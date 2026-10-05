//! `MsgSecretStore` for the SQL family. New in whatsapp-rust 0.7, where it
//! became a required supertrait of `Backend`. Stores the 32-byte
//! `messageSecret` keyed by the outbound message it rode on, never the message
//! itself.

use async_trait::async_trait;
use wacore::store::error::Result;
use wacore::store::traits::{MsgSecretEntry, MsgSecretStore};

use super::{SqlBackend, SqlTx};
use crate::storage::statements::msg_secret::{
    DELETE_EXPIRED_MSG_SECRETS, GET_MSG_SECRET, GET_MSG_SECRET_WITH_TS, PUT_MSG_SECRET,
};

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
            execute_sql!(
                in tx,
                PUT_MSG_SECRET,
                &*e.chat,
                &*e.sender,
                &*e.msg_id,
                e.secret.as_slice(),
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
        scalar_optional_sql!(
            Vec<u8>,
            &self.pool,
            GET_MSG_SECRET,
            chat,
            sender,
            msg_id,
            self.device_id
        )
    }

    /// Overridden rather than defaulted: the default pairs the secret with a
    /// hardcoded `0`, which would silently disable the edit-processing window.
    async fn get_msg_secret_with_ts(
        &self,
        chat: &str,
        sender: &str,
        msg_id: &str,
    ) -> Result<Option<(Vec<u8>, i64)>> {
        row_optional_sql!(
            (Vec<u8>, i64),
            &self.pool,
            GET_MSG_SECRET_WITH_TS,
            chat,
            sender,
            msg_id,
            self.device_id
        )
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
