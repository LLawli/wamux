//! `ProtocolStore` for the SQL family: per-device sender-key tracking, LID-PN
//! mappings, base keys, device registry, tc tokens, and the sent-message cache.

use async_trait::async_trait;
use wacore::store::error::Result;
use wacore::store::traits::{DeviceListRecord, LidPnMappingEntry, ProtocolStore, TcTokenEntry};

use super::protocol_batch_sql;
use super::{SqlBackend, SqlTx, tc_token_sql};
use crate::storage::blob_cipher::joined_row;
use crate::storage::blob_codec::now_secs;
use crate::storage::protocol_rows::{self, DeviceListRow, LidMappingRow};
use crate::storage::statements::protocol::{
    CLEAR_ALL_SENDER_KEY_DEVICES, CLEAR_SENDER_KEY_DEVICES, DELETE_BASE_KEY, DELETE_DEVICES,
    DELETE_EXPIRED_BASE_KEYS, DELETE_EXPIRED_SENT_MESSAGES, DELETE_EXPIRED_TC_TOKENS,
    DELETE_SENDER_KEY_DEVICE_ROW, DELETE_SENT_MESSAGE, DELETE_TC_TOKEN, GET_ALL_LID_MAPPINGS,
    GET_ALL_TC_TOKEN_JIDS, GET_BASE_KEY, GET_DEVICES, GET_LID_MAPPING, GET_PN_MAPPING,
    GET_SENDER_KEY_DEVICES, GET_SENT_MESSAGE, GET_TC_TOKEN, PUT_LID_MAPPING, PUT_TC_TOKEN,
    SAVE_BASE_KEY, SET_SENDER_KEY_STATUS, STORE_SENT_MESSAGE, UPDATE_DEVICE_LIST,
};

impl SqlBackend {
    /// Open a `sent_messages.payload` read for `(chat_jid, message_id)`.
    fn open_sent(
        &self,
        chat_jid: &str,
        message_id: &str,
        stored: Option<Vec<u8>>,
    ) -> Result<Option<Vec<u8>>> {
        let row = joined_row(&[chat_jid.as_bytes(), message_id.as_bytes()]);
        stored
            .map(|stored| self.open_at("sent_messages", "payload", &row, &stored))
            .transpose()
    }
}

#[async_trait]
impl ProtocolStore for SqlBackend {
    // --- Per-device sender key tracking ---

    async fn get_sender_key_devices(&self, group_jid: &str) -> Result<Vec<(String, bool)>> {
        let rows = row_all_sql!(
            (String, i32),
            &self.pool,
            GET_SENDER_KEY_DEVICES,
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
                SET_SENDER_KEY_STATUS,
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
            CLEAR_SENDER_KEY_DEVICES,
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
                DELETE_SENDER_KEY_DEVICE_ROW,
                device_jid,
                self.device_id
            )?;
        }
        tx.commit().await
    }

    async fn clear_all_sender_key_devices(&self) -> Result<()> {
        execute_sql!(&self.pool, CLEAR_ALL_SENDER_KEY_DEVICES, self.device_id)?;
        Ok(())
    }

    // --- LID-PN mapping ---

    async fn get_lid_mapping(&self, lid: &str) -> Result<Option<LidPnMappingEntry>> {
        let row = row_optional_sql!(
            LidMappingRow,
            &self.pool,
            GET_LID_MAPPING,
            lid,
            self.device_id
        )?;
        Ok(row.map(protocol_rows::lid_entry))
    }

    async fn get_pn_mapping(&self, phone: &str) -> Result<Option<LidPnMappingEntry>> {
        let row = row_optional_sql!(
            LidMappingRow,
            &self.pool,
            GET_PN_MAPPING,
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
            GET_ALL_LID_MAPPINGS,
            self.device_id
        )?;
        Ok(rows.into_iter().map(protocol_rows::lid_entry).collect())
    }

    // --- Base key collision detection ---

    async fn save_base_key(&self, address: &str, message_id: &str, base_key: &[u8]) -> Result<()> {
        let row = joined_row(&[address.as_bytes(), message_id.as_bytes()]);
        let sealed = self.seal_at("base_keys", "base_key", &row, base_key)?;
        execute_sql!(
            &self.pool,
            SAVE_BASE_KEY,
            address,
            message_id,
            &sealed[..],
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
            GET_BASE_KEY,
            address,
            message_id,
            self.device_id
        )?;
        // Open before comparing: the stored bytes are sealed, so a byte compare
        // against the plain key would never match.
        let row_key = joined_row(&[address.as_bytes(), message_id.as_bytes()]);
        let stored = row
            .map(|stored| self.open_at("base_keys", "base_key", &row_key, &stored))
            .transpose()?;
        Ok(stored.as_deref() == Some(current_base_key))
    }

    async fn delete_base_key(&self, address: &str, message_id: &str) -> Result<()> {
        execute_sql!(
            &self.pool,
            DELETE_BASE_KEY,
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
        let row = row_optional_sql!(DeviceListRow, &self.pool, GET_DEVICES, user, self.device_id)?;
        row.map(protocol_rows::device_list_record).transpose()
    }

    async fn delete_devices(&self, user: &str) -> Result<()> {
        execute_sql!(&self.pool, DELETE_DEVICES, user, self.device_id)?;
        Ok(())
    }

    // --- TcToken storage ---

    async fn get_tc_token(&self, jid: &str) -> Result<Option<TcTokenEntry>> {
        let row = row_optional_sql!(
            (Vec<u8>, i64, Option<i64>),
            &self.pool,
            GET_TC_TOKEN,
            jid,
            self.device_id
        )?;
        row.map(|(stored, token_timestamp, sender_timestamp)| {
            let token = self.open_at("tc_tokens", "token", jid.as_bytes(), &stored)?;
            Ok(TcTokenEntry {
                token,
                token_timestamp,
                sender_timestamp,
            })
        })
        .transpose()
    }

    async fn put_tc_token(&self, jid: &str, entry: &TcTokenEntry) -> Result<()> {
        let sealed = self.seal_at("tc_tokens", "token", jid.as_bytes(), &entry.token)?;
        execute_sql!(
            &self.pool,
            PUT_TC_TOKEN,
            jid,
            &sealed[..],
            entry.token_timestamp,
            entry.sender_timestamp,
            self.device_id,
            now_secs()
        )?;
        Ok(())
    }

    async fn delete_tc_token(&self, jid: &str) -> Result<()> {
        execute_sql!(&self.pool, DELETE_TC_TOKEN, jid, self.device_id)?;
        Ok(())
    }

    async fn get_all_tc_token_jids(&self) -> Result<Vec<String>> {
        scalar_all_sql!(String, &self.pool, GET_ALL_TC_TOKEN_JIDS, self.device_id)
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
            DELETE_EXPIRED_TC_TOKENS,
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
        let row = joined_row(&[chat_jid.as_bytes(), message_id.as_bytes()]);
        let sealed = self.seal_at("sent_messages", "payload", &row, payload)?;
        execute_sql!(
            &self.pool,
            STORE_SENT_MESSAGE,
            chat_jid,
            message_id,
            &sealed[..],
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
            GET_SENT_MESSAGE,
            chat_jid,
            message_id,
            self.device_id
        )?;
        if payload.is_some() {
            execute_sql!(
                in tx,
                DELETE_SENT_MESSAGE,
                chat_jid,
                message_id,
                self.device_id
            )?;
        }
        // Open before committing: a blob that will not open must not be consumed.
        let payload = self.open_sent(chat_jid, message_id, payload)?;
        tx.commit().await?;
        Ok(payload)
    }

    async fn delete_expired_sent_messages(&self, cutoff_timestamp: i64) -> Result<u32> {
        let deleted = execute_sql!(
            &self.pool,
            DELETE_EXPIRED_SENT_MESSAGES,
            cutoff_timestamp,
            self.device_id
        )?;
        Ok(deleted as u32)
    }

    // --- Trait defaults overridden in #93 (docs/store-trait-defaults.md) ---

    async fn get_sent_message(&self, chat_jid: &str, message_id: &str) -> Result<Option<Vec<u8>>> {
        // Read-only on purpose: `take_sent_message` consumes, this must not.
        let payload = scalar_optional_sql!(
            Vec<u8>,
            &self.pool,
            GET_SENT_MESSAGE,
            chat_jid,
            message_id,
            self.device_id
        )?;
        self.open_sent(chat_jid, message_id, payload)
    }

    async fn delete_expired_base_keys(&self, cutoff_timestamp: i64) -> Result<u32> {
        // The keepalive sweep calls this every cycle; the trait default was Ok(0),
        // so base_keys grew without bound.
        let deleted = execute_sql!(
            &self.pool,
            DELETE_EXPIRED_BASE_KEYS,
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
        tc_token_sql::store_received(
            &self.pool,
            &self.cipher,
            self.device_id,
            jid,
            token,
            token_timestamp,
        )
        .await
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
        protocol_batch_sql::get_tc_tokens(&self.pool, &self.cipher, self.device_id, jids).await
    }
}
