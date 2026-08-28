//! `MsgSecretStore` over SQLite. New in whatsapp-rust 0.7, where it became a
//! required supertrait of `Backend`. Stores the 32-byte `messageSecret` keyed by
//! the outbound message it rode on, never the message itself.

use async_trait::async_trait;
use wacore::store::error::Result;
use wacore::store::traits::{MsgSecretEntry, MsgSecretStore};

use super::SqliteBackend;
use crate::storage::sqlx_error::db;

#[async_trait]
impl MsgSecretStore for SqliteBackend {
    /// Upsert must never SHORTEN a retention window: `expires_at = 0` means
    /// "never" and beats any deadline, otherwise the later deadline wins. The
    /// same guard applies to `message_ts`, where a `0` ("unknown") must not
    /// clobber a parent time we already learned. Both rules are the lib's
    /// `merge_msg_secret_*` helpers, expressed in SQL so one statement per row
    /// stays atomic against a concurrent redelivery. `MAX` is sqlite's spelling
    /// of `GREATEST`.
    async fn put_msg_secrets(&self, entries: Vec<MsgSecretEntry>) -> Result<usize> {
        if entries.is_empty() {
            return Ok(0);
        }
        let mut tx = self.pool.begin().await.map_err(db)?;
        for e in &entries {
            sqlx::query(
                "INSERT INTO msg_secrets
                     (chat, sender, msg_id, secret, expires_at, message_ts, device_id)
                 VALUES (?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT (chat, sender, msg_id, device_id) DO UPDATE SET
                     secret     = excluded.secret,
                     expires_at = CASE
                         WHEN msg_secrets.expires_at = 0 OR excluded.expires_at = 0 THEN 0
                         ELSE MAX(msg_secrets.expires_at, excluded.expires_at)
                     END,
                     message_ts = MAX(msg_secrets.message_ts, excluded.message_ts)",
            )
            .bind(&*e.chat)
            .bind(&*e.sender)
            .bind(&*e.msg_id)
            .bind(e.secret.as_slice())
            .bind(e.expires_at)
            .bind(e.message_ts)
            .bind(self.device_id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        }
        tx.commit().await.map_err(db)?;
        Ok(entries.len())
    }

    async fn get_msg_secret(
        &self,
        chat: &str,
        sender: &str,
        msg_id: &str,
    ) -> Result<Option<Vec<u8>>> {
        let row: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT secret FROM msg_secrets
             WHERE chat = ? AND sender = ? AND msg_id = ? AND device_id = ?",
        )
        .bind(chat)
        .bind(sender)
        .bind(msg_id)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row)
    }

    /// Overridden rather than defaulted: the default pairs the secret with a
    /// hardcoded `0`, which would silently disable the edit-processing window.
    async fn get_msg_secret_with_ts(
        &self,
        chat: &str,
        sender: &str,
        msg_id: &str,
    ) -> Result<Option<(Vec<u8>, i64)>> {
        let row: Option<(Vec<u8>, i64)> = sqlx::query_as(
            "SELECT secret, message_ts FROM msg_secrets
             WHERE chat = ? AND sender = ? AND msg_id = ? AND device_id = ?",
        )
        .bind(chat)
        .bind(sender)
        .bind(msg_id)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row)
    }

    /// `expires_at = 0` is "never" and is deliberately excluded from the sweep.
    async fn delete_expired_msg_secrets(&self, cutoff_timestamp: i64) -> Result<u32> {
        let res = sqlx::query(
            "DELETE FROM msg_secrets
             WHERE expires_at <> 0 AND expires_at <= ? AND device_id = ?",
        )
        .bind(cutoff_timestamp)
        .bind(self.device_id)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(res.rows_affected() as u32)
    }
}
