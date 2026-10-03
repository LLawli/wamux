//! `MsgSecretStore` for the turso family, method for method the sql family's
//! (#106). Stores the 32-byte `messageSecret` keyed by the outbound message it
//! rode on, never the message itself.

use async_trait::async_trait;
use wacore::store::error::Result;
use wacore::store::traits::{MsgSecretEntry, MsgSecretStore};

use super::exec::binds;
use super::row_values::{blob, first_blob, int};
use super::{TursoBackend, TursoTx};
use crate::storage::statements::msg_secret::{
    DELETE_EXPIRED_MSG_SECRETS, GET_MSG_SECRET, GET_MSG_SECRET_WITH_TS, PUT_MSG_SECRET,
};

#[async_trait]
impl MsgSecretStore for TursoBackend {
    /// One upsert per row in one transaction. The never-shorten-a-retention-window
    /// rules live with the statement (`statements::msg_secret::PUT_MSG_SECRET`).
    async fn put_msg_secrets(&self, entries: Vec<MsgSecretEntry>) -> Result<usize> {
        if entries.is_empty() {
            return Ok(0);
        }
        let tx = TursoTx::begin(&self.conn).await?;
        for e in &entries {
            let binds = binds![
                &*e.chat,
                &*e.sender,
                &*e.msg_id,
                e.secret.as_slice(),
                e.expires_at,
                e.message_ts,
                self.device_id
            ];
            tx.execute(PUT_MSG_SECRET, binds).await?;
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
        let binds = binds![chat, sender, msg_id, self.device_id];
        first_blob(self.conn.fetch_optional(GET_MSG_SECRET, binds).await?)
    }

    /// Overridden rather than defaulted: the default pairs the secret with a
    /// hardcoded `0`, which would silently disable the edit-processing window.
    async fn get_msg_secret_with_ts(
        &self,
        chat: &str,
        sender: &str,
        msg_id: &str,
    ) -> Result<Option<(Vec<u8>, i64)>> {
        let binds = binds![chat, sender, msg_id, self.device_id];
        let row = self
            .conn
            .fetch_optional(GET_MSG_SECRET_WITH_TS, binds)
            .await?;
        row.map(|row| Ok((blob(&row, 0)?, int(&row, 1)?)))
            .transpose()
    }

    /// `expires_at = 0` is "never" and is deliberately excluded from the sweep.
    async fn delete_expired_msg_secrets(&self, cutoff_timestamp: i64) -> Result<u32> {
        let binds = binds![cutoff_timestamp, self.device_id];
        let deleted = self.conn.execute(DELETE_EXPIRED_MSG_SECRETS, binds).await?;
        Ok(deleted as u32)
    }
}
