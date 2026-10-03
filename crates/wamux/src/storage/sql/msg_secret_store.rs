//! `MsgSecretStore` for the SQL family. New in whatsapp-rust 0.7, where it
//! became a required supertrait of `Backend`. Stores the 32-byte
//! `messageSecret` keyed by the outbound message it rode on, never the message
//! itself.

use async_trait::async_trait;
use wacore::store::error::Result;
use wacore::store::traits::{MsgSecretEntry, MsgSecretStore};

use super::{SqlBackend, SqlTx};

#[async_trait]
impl MsgSecretStore for SqlBackend {
    /// Upsert must never SHORTEN a retention window: `expires_at = 0` means
    /// "never" and beats any deadline, otherwise the later deadline wins. The
    /// same guard applies to `message_ts`, where a `0` ("unknown") must not
    /// clobber a parent time we already learned. Both rules are the lib's
    /// `merge_msg_secret_*` helpers, expressed in SQL so one statement per row
    /// stays atomic against a concurrent redelivery. The later-wins pick is a
    /// `CASE` because `GREATEST` (Postgres) and two-argument `MAX` (SQLite) do
    /// not spell the same way; all the columns involved are NOT NULL.
    async fn put_msg_secrets(&self, entries: Vec<MsgSecretEntry>) -> Result<usize> {
        if entries.is_empty() {
            return Ok(0);
        }
        let mut tx = SqlTx::begin(&self.pool).await?;
        for e in &entries {
            execute_sql!(
                in tx,
                "INSERT INTO msg_secrets
                     (chat, sender, msg_id, secret, expires_at, message_ts, device_id)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT (chat, sender, msg_id, device_id) DO UPDATE SET
                     secret     = EXCLUDED.secret,
                     expires_at = CASE
                         WHEN msg_secrets.expires_at = 0 OR EXCLUDED.expires_at = 0 THEN 0
                         WHEN msg_secrets.expires_at > EXCLUDED.expires_at
                             THEN msg_secrets.expires_at
                         ELSE EXCLUDED.expires_at
                     END,
                     message_ts = CASE
                         WHEN msg_secrets.message_ts > EXCLUDED.message_ts
                             THEN msg_secrets.message_ts
                         ELSE EXCLUDED.message_ts
                     END",
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
            "SELECT secret FROM msg_secrets
             WHERE chat = $1 AND sender = $2 AND msg_id = $3 AND device_id = $4",
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
            "SELECT secret, message_ts FROM msg_secrets
             WHERE chat = $1 AND sender = $2 AND msg_id = $3 AND device_id = $4",
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
            "DELETE FROM msg_secrets
             WHERE expires_at <> 0 AND expires_at <= $1 AND device_id = $2",
            cutoff_timestamp,
            self.device_id
        )?;
        Ok(deleted as u32)
    }
}
