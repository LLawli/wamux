//! `MsgSecretStore` for the SQL family. New in whatsapp-rust 0.7, where it
//! became a required supertrait of `Backend`. Stores the 32-byte
//! `messageSecret` keyed by the outbound message it rode on, never the message
//! itself.

use async_trait::async_trait;
use wacore::store::error::Result;
use wacore::store::traits::{MsgSecretEntry, MsgSecretStore};

use super::{SqlBackend, SqlTx};
use crate::storage::blob_cipher::joined_row;
use crate::storage::statements::msg_secret::{
    DELETE_EXPIRED_MSG_SECRETS, GET_MSG_SECRET, GET_MSG_SECRET_WITH_TS, PUT_MSG_SECRET,
};

/// The AAD row key of `msg_secrets`: `chat | 0 | sender | 0 | msg_id`.
fn secret_row(chat: &str, sender: &str, msg_id: &str) -> Vec<u8> {
    joined_row(&[chat.as_bytes(), sender.as_bytes(), msg_id.as_bytes()])
}

#[async_trait]
impl MsgSecretStore for SqlBackend {
    /// One upsert per row in one transaction. The never-shorten-a-retention-window
    /// rules live with the statement (`statements::msg_secret::PUT_MSG_SECRET`).
    async fn put_msg_secrets(&self, entries: Vec<MsgSecretEntry>) -> Result<usize> {
        if entries.is_empty() {
            return Ok(0);
        }
        let mut tx = SqlTx::begin(&self.pool).await?;
        for e in &entries {
            let row = secret_row(&e.chat, &e.sender, &e.msg_id);
            let sealed = self.seal_at("msg_secrets", "secret", &row, e.secret.as_slice())?;
            execute_sql!(
                in tx,
                PUT_MSG_SECRET,
                &*e.chat,
                &*e.sender,
                &*e.msg_id,
                &sealed[..],
                e.expires_at,
                e.message_ts,
                self.device_id
            )?;
        }
        tx.commit().await?;
        Ok(entries.len())
    }

    async fn get_msg_secret(
        &self,
        chat: &str,
        sender: &str,
        msg_id: &str,
    ) -> Result<Option<Vec<u8>>> {
        let stored = scalar_optional_sql!(
            Vec<u8>,
            &self.pool,
            GET_MSG_SECRET,
            chat,
            sender,
            msg_id,
            self.device_id
        )?;
        let row = secret_row(chat, sender, msg_id);
        stored
            .map(|stored| self.open_at("msg_secrets", "secret", &row, &stored))
            .transpose()
    }

    /// Overridden rather than defaulted: the default pairs the secret with a
    /// hardcoded `0`, which would silently disable the edit-processing window.
    async fn get_msg_secret_with_ts(
        &self,
        chat: &str,
        sender: &str,
        msg_id: &str,
    ) -> Result<Option<(Vec<u8>, i64)>> {
        let found = row_optional_sql!(
            (Vec<u8>, i64),
            &self.pool,
            GET_MSG_SECRET_WITH_TS,
            chat,
            sender,
            msg_id,
            self.device_id
        )?;
        let row = secret_row(chat, sender, msg_id);
        found
            .map(|(stored, ts)| Ok((self.open_at("msg_secrets", "secret", &row, &stored)?, ts)))
            .transpose()
    }

    /// `expires_at = 0` is "never" and is deliberately excluded from the sweep.
    async fn delete_expired_msg_secrets(&self, cutoff_timestamp: i64) -> Result<u32> {
        let deleted = execute_sql!(
            &self.pool,
            DELETE_EXPIRED_MSG_SECRETS,
            cutoff_timestamp,
            self.device_id
        )?;
        Ok(deleted as u32)
    }
}
