//! `ProtocolStore` for the SQL family: per-device sender-key tracking, LID-PN
//! mappings, base keys, device registry, tc tokens, and the sent-message cache.

use async_trait::async_trait;
use wacore::store::error::Result;
use wacore::store::traits::{DeviceListRecord, LidPnMappingEntry, ProtocolStore, TcTokenEntry};

use super::protocol_batch_sql::{self, PUT_LID_MAPPING, UPDATE_DEVICE_LIST};
use super::protocol_rows::{self, DeviceListRow, LidMappingRow};
use super::{SqlBackend, SqlTx, tc_token_sql};
use crate::storage::blob_codec::now_secs;

#[async_trait]
impl ProtocolStore for SqlBackend {
    // --- Per-device sender key tracking ---

    async fn get_sender_key_devices(&self, group_jid: &str) -> Result<Vec<(String, bool)>> {
        let rows = row_all_sql!(
            (String, i32),
            &self.pool,
            "SELECT device_jid, has_key FROM sender_key_devices
             WHERE group_jid = $1 AND device_id = $2",
            group_jid,
            self.device_id
        )?;
        Ok(rows.into_iter().map(|(jid, has)| (jid, has != 0)).collect())
    }

    async fn set_sender_key_status(&self, group_jid: &str, entries: &[(&str, bool)]) -> Result<()> {
        let now = now_secs();
        let mut tx = SqlTx::begin(&self.pool).await?;
        for (device_jid, has_key) in entries {
            execute_sql!(
                in tx,
                "INSERT INTO sender_key_devices (group_jid, device_jid, has_key, device_id, updated_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (group_jid, device_jid, device_id)
                 DO UPDATE SET has_key = EXCLUDED.has_key, updated_at = EXCLUDED.updated_at",
                group_jid,
                device_jid,
                i32::from(*has_key),
                self.device_id,
                now
            )?;
        }
        tx.commit().await
    }

    async fn clear_sender_key_devices(&self, group_jid: &str) -> Result<()> {
        execute_sql!(
            &self.pool,
            "DELETE FROM sender_key_devices WHERE group_jid = $1 AND device_id = $2",
            group_jid,
            self.device_id
        )?;
        Ok(())
    }

    async fn delete_sender_key_device_rows(&self, device_jids: &[&str]) -> Result<()> {
        let mut tx = SqlTx::begin(&self.pool).await?;
        for device_jid in device_jids {
            execute_sql!(
                in tx,
                "DELETE FROM sender_key_devices WHERE device_jid = $1 AND device_id = $2",
                device_jid,
                self.device_id
            )?;
        }
        tx.commit().await
    }

    async fn clear_all_sender_key_devices(&self) -> Result<()> {
        execute_sql!(
            &self.pool,
            "DELETE FROM sender_key_devices WHERE device_id = $1",
            self.device_id
        )?;
        Ok(())
    }

    // --- LID-PN mapping ---

    async fn get_lid_mapping(&self, lid: &str) -> Result<Option<LidPnMappingEntry>> {
        let row = row_optional_sql!(
            LidMappingRow,
            &self.pool,
            "SELECT lid, phone_number, created_at, learning_source, updated_at
             FROM lid_pn_mapping WHERE lid = $1 AND device_id = $2",
            lid,
            self.device_id
        )?;
        Ok(row.map(protocol_rows::lid_entry))
    }

    async fn get_pn_mapping(&self, phone: &str) -> Result<Option<LidPnMappingEntry>> {
        let row = row_optional_sql!(
            LidMappingRow,
            &self.pool,
            "SELECT lid, phone_number, created_at, learning_source, updated_at
             FROM lid_pn_mapping WHERE phone_number = $1 AND device_id = $2
             ORDER BY updated_at DESC LIMIT 1",
            phone,
            self.device_id
        )?;
        Ok(row.map(protocol_rows::lid_entry))
    }

    async fn put_lid_mapping(&self, entry: &LidPnMappingEntry) -> Result<()> {
        execute_sql!(
            &self.pool,
            PUT_LID_MAPPING,
            &entry.lid,
            &entry.phone_number,
            entry.created_at,
            &entry.learning_source,
            entry.updated_at,
            self.device_id
        )?;
        Ok(())
    }

    async fn get_all_lid_mappings(&self) -> Result<Vec<LidPnMappingEntry>> {
        let rows = row_all_sql!(
            LidMappingRow,
            &self.pool,
            "SELECT lid, phone_number, created_at, learning_source, updated_at
             FROM lid_pn_mapping WHERE device_id = $1",
            self.device_id
        )?;
        Ok(rows.into_iter().map(protocol_rows::lid_entry).collect())
    }

    // --- Base key collision detection ---

    async fn save_base_key(&self, address: &str, message_id: &str, base_key: &[u8]) -> Result<()> {
        execute_sql!(
            &self.pool,
            "INSERT INTO base_keys (address, message_id, base_key, device_id, created_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (address, message_id, device_id) DO UPDATE SET base_key = EXCLUDED.base_key",
            address,
            message_id,
            base_key,
            self.device_id,
            now_secs()
        )?;
        Ok(())
    }

    async fn has_same_base_key(
        &self,
        address: &str,
        message_id: &str,
        current_base_key: &[u8],
    ) -> Result<bool> {
        let row = scalar_optional_sql!(
            Vec<u8>,
            &self.pool,
            "SELECT base_key FROM base_keys
             WHERE address = $1 AND message_id = $2 AND device_id = $3",
            address,
            message_id,
            self.device_id
        )?;
        Ok(row.as_deref() == Some(current_base_key))
    }

    async fn delete_base_key(&self, address: &str, message_id: &str) -> Result<()> {
        execute_sql!(
            &self.pool,
            "DELETE FROM base_keys WHERE address = $1 AND message_id = $2 AND device_id = $3",
            address,
            message_id,
            self.device_id
        )?;
        Ok(())
    }

    // --- Device registry ---

    async fn update_device_list(&self, record: DeviceListRecord) -> Result<()> {
        let devices_json = protocol_rows::devices_json(&record)?;
        // sqlx has no `Encode` for `Arc<str>` / `Box<str>`, so both are
        // deref'd to `&str` at the bind site.
        execute_sql!(
            &self.pool,
            UPDATE_DEVICE_LIST,
            &*record.user,
            &devices_json,
            record.timestamp,
            record.phash.as_deref(),
            self.device_id,
            now_secs(),
            record.raw_id.map(|r| r as i32)
        )?;
        Ok(())
    }

    async fn get_devices(&self, user: &str) -> Result<Option<DeviceListRecord>> {
        let row = row_optional_sql!(
            DeviceListRow,
            &self.pool,
            "SELECT user_id, devices_json, timestamp, phash, raw_id
             FROM device_registry WHERE user_id = $1 AND device_id = $2",
            user,
            self.device_id
        )?;
        row.map(protocol_rows::device_list_record).transpose()
    }

    async fn delete_devices(&self, user: &str) -> Result<()> {
        execute_sql!(
            &self.pool,
            "DELETE FROM device_registry WHERE user_id = $1 AND device_id = $2",
            user,
            self.device_id
        )?;
        Ok(())
    }

    // --- TcToken storage ---

    async fn get_tc_token(&self, jid: &str) -> Result<Option<TcTokenEntry>> {
        let row = row_optional_sql!(
            (Vec<u8>, i64, Option<i64>),
            &self.pool,
            "SELECT token, token_timestamp, sender_timestamp
             FROM tc_tokens WHERE jid = $1 AND device_id = $2",
            jid,
            self.device_id
        )?;
        Ok(
            row.map(|(token, token_timestamp, sender_timestamp)| TcTokenEntry {
                token,
                token_timestamp,
                sender_timestamp,
            }),
        )
    }

    async fn put_tc_token(&self, jid: &str, entry: &TcTokenEntry) -> Result<()> {
        execute_sql!(
            &self.pool,
            "INSERT INTO tc_tokens
                (jid, token, token_timestamp, sender_timestamp, device_id, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (jid, device_id) DO UPDATE SET
                token = EXCLUDED.token,
                token_timestamp = EXCLUDED.token_timestamp,
                sender_timestamp = EXCLUDED.sender_timestamp,
                updated_at = EXCLUDED.updated_at",
            jid,
            entry.token.as_slice(),
            entry.token_timestamp,
            entry.sender_timestamp,
            self.device_id,
            now_secs()
        )?;
        Ok(())
    }

    async fn delete_tc_token(&self, jid: &str) -> Result<()> {
        execute_sql!(
            &self.pool,
            "DELETE FROM tc_tokens WHERE jid = $1 AND device_id = $2",
            jid,
            self.device_id
        )?;
        Ok(())
    }

    async fn get_all_tc_token_jids(&self) -> Result<Vec<String>> {
        scalar_all_sql!(
            String,
            &self.pool,
            "SELECT jid FROM tc_tokens WHERE device_id = $1",
            self.device_id
        )
    }

    /// Two independent windows, both of which must be stale before the row goes:
    /// the received token (expired or byte-empty) AND the sender bucket (expired
    /// or never set). Pruning on the token alone would drop recent sender state
    /// that the retry path still needs. Mirrors the reference `SqliteStore`.
    /// `length()` counts bytes for a Postgres `bytea` too, so it stands in for the
    /// Postgres-only byte-length function.
    async fn delete_expired_tc_tokens(&self, token_cutoff: i64, sender_cutoff: i64) -> Result<u32> {
        let deleted = execute_sql!(
            &self.pool,
            "DELETE FROM tc_tokens
             WHERE (length(token) = 0 OR token_timestamp < $1)
               AND (sender_timestamp IS NULL OR sender_timestamp < $2)
               AND device_id = $3",
            token_cutoff,
            sender_cutoff,
            self.device_id
        )?;
        Ok(deleted as u32)
    }

    // --- Sent message cache (retry support) ---

    async fn store_sent_message(
        &self,
        chat_jid: &str,
        message_id: &str,
        payload: &[u8],
    ) -> Result<()> {
        // REPLACE semantics in the reference reset created_at; mirror that.
        execute_sql!(
            &self.pool,
            "INSERT INTO sent_messages (chat_jid, message_id, payload, device_id, created_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (chat_jid, message_id, device_id)
             DO UPDATE SET payload = EXCLUDED.payload, created_at = EXCLUDED.created_at",
            chat_jid,
            message_id,
            payload,
            self.device_id,
            now_secs()
        )?;
        Ok(())
    }

    async fn take_sent_message(&self, chat_jid: &str, message_id: &str) -> Result<Option<Vec<u8>>> {
        let mut tx = SqlTx::begin(&self.pool).await?;
        let payload = scalar_optional_sql!(
            Vec<u8>,
            in tx,
            "SELECT payload FROM sent_messages
             WHERE chat_jid = $1 AND message_id = $2 AND device_id = $3",
            chat_jid,
            message_id,
            self.device_id
        )?;
        if payload.is_some() {
            execute_sql!(
                in tx,
                "DELETE FROM sent_messages
                 WHERE chat_jid = $1 AND message_id = $2 AND device_id = $3",
                chat_jid,
                message_id,
                self.device_id
            )?;
        }
        tx.commit().await?;
        Ok(payload)
    }

    async fn delete_expired_sent_messages(&self, cutoff_timestamp: i64) -> Result<u32> {
        let deleted = execute_sql!(
            &self.pool,
            "DELETE FROM sent_messages WHERE created_at < $1 AND device_id = $2",
            cutoff_timestamp,
            self.device_id
        )?;
        Ok(deleted as u32)
    }

    // --- Trait defaults overridden in #93 (docs/store-trait-defaults.md) ---

    async fn get_sent_message(&self, chat_jid: &str, message_id: &str) -> Result<Option<Vec<u8>>> {
        // Read-only on purpose: `take_sent_message` consumes, this must not.
        scalar_optional_sql!(
            Vec<u8>,
            &self.pool,
            "SELECT payload FROM sent_messages
             WHERE chat_jid = $1 AND message_id = $2 AND device_id = $3",
            chat_jid,
            message_id,
            self.device_id
        )
    }

    async fn delete_expired_base_keys(&self, cutoff_timestamp: i64) -> Result<u32> {
        // The keepalive sweep calls this every cycle; the trait default was Ok(0),
        // so base_keys grew without bound.
        let deleted = execute_sql!(
            &self.pool,
            "DELETE FROM base_keys WHERE created_at < $1 AND device_id = $2",
            cutoff_timestamp,
            self.device_id
        )?;
        Ok(deleted as u32)
    }

    async fn touch_tc_token_sender_timestamp(
        &self,
        jid: &str,
        sender_timestamp: i64,
    ) -> Result<()> {
        tc_token_sql::touch_sender_timestamp(&self.pool, self.device_id, jid, sender_timestamp)
            .await
    }

    async fn store_received_tc_token(
        &self,
        jid: &str,
        token: &[u8],
        token_timestamp: i64,
    ) -> Result<()> {
        tc_token_sql::store_received(&self.pool, self.device_id, jid, token, token_timestamp).await
    }

    // --- Throughput overrides (#104), see `protocol_batch_sql` ---

    async fn put_lid_mappings(&self, entries: &[LidPnMappingEntry]) -> Result<()> {
        protocol_batch_sql::put_lid_mappings(&self.pool, self.device_id, entries).await
    }

    async fn update_device_lists(&self, records: Vec<DeviceListRecord>) -> Result<()> {
        protocol_batch_sql::update_device_lists(&self.pool, self.device_id, records).await
    }

    async fn get_devices_batch(&self, users: &[&str]) -> Result<Vec<DeviceListRecord>> {
        protocol_batch_sql::get_devices_batch(&self.pool, self.device_id, users).await
    }

    async fn get_tc_tokens(&self, jids: &[String]) -> Result<Vec<Option<TcTokenEntry>>> {
        protocol_batch_sql::get_tc_tokens(&self.pool, self.device_id, jids).await
    }
}
