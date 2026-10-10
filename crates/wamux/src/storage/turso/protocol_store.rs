//! `ProtocolStore` for the turso family, method for method the sql family's
//! (#106): per-device sender-key tracking, LID-PN mappings, base keys, device
//! registry, tc tokens, and the sent-message cache.

use async_trait::async_trait;
use wacore::store::error::Result;
use wacore::store::traits::{DeviceListRecord, LidPnMappingEntry, ProtocolStore, TcTokenEntry};

use super::exec::binds;
use super::protocol_decode::{device_list_row, lid_mapping_row, sender_key_device, tc_token_entry};
use super::row_values::{first_blob, text};
use super::{TursoBackend, TursoTx, protocol_batch_sql, tc_token_sql};
use crate::storage::blob_cipher::joined_row;
use crate::storage::blob_codec::now_secs;
use crate::storage::protocol_rows;
use crate::storage::statements::protocol::{
    CLEAR_ALL_SENDER_KEY_DEVICES, CLEAR_SENDER_KEY_DEVICES, DELETE_BASE_KEY, DELETE_DEVICES,
    DELETE_EXPIRED_BASE_KEYS, DELETE_EXPIRED_SENT_MESSAGES, DELETE_EXPIRED_TC_TOKENS,
    DELETE_SENDER_KEY_DEVICE_ROW, DELETE_SENT_MESSAGE, DELETE_TC_TOKEN, GET_ALL_LID_MAPPINGS,
    GET_ALL_TC_TOKEN_JIDS, GET_BASE_KEY, GET_DEVICES, GET_LID_MAPPING, GET_PN_MAPPING,
    GET_SENDER_KEY_DEVICES, GET_SENT_MESSAGE, GET_TC_TOKEN, PUT_LID_MAPPING, PUT_TC_TOKEN,
    SAVE_BASE_KEY, SET_SENDER_KEY_STATUS, STORE_SENT_MESSAGE, UPDATE_DEVICE_LIST,
};

impl TursoBackend {
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
impl ProtocolStore for TursoBackend {
    // --- Per-device sender key tracking ---

    async fn get_sender_key_devices(&self, group_jid: &str) -> Result<Vec<(String, bool)>> {
        let binds = binds![group_jid, self.device_id];
        let rows = self.conn.fetch_all(GET_SENDER_KEY_DEVICES, binds).await?;
        rows.iter().map(sender_key_device).collect()
    }

    async fn set_sender_key_status(&self, group_jid: &str, entries: &[(&str, bool)]) -> Result<()> {
        let now = now_secs();
        let tx = TursoTx::begin(&self.conn).await?;
        for (device_jid, has_key) in entries {
            let binds = binds![
                group_jid,
                *device_jid,
                i32::from(*has_key),
                self.device_id,
                now
            ];
            tx.execute(SET_SENDER_KEY_STATUS, binds).await?;
        }
        tx.commit().await
    }

    async fn clear_sender_key_devices(&self, group_jid: &str) -> Result<()> {
        let binds = binds![group_jid, self.device_id];
        self.conn.execute(CLEAR_SENDER_KEY_DEVICES, binds).await?;
        Ok(())
    }

    async fn delete_sender_key_device_rows(&self, device_jids: &[&str]) -> Result<()> {
        let tx = TursoTx::begin(&self.conn).await?;
        for device_jid in device_jids {
            let binds = binds![*device_jid, self.device_id];
            tx.execute(DELETE_SENDER_KEY_DEVICE_ROW, binds).await?;
        }
        tx.commit().await
    }

    async fn clear_all_sender_key_devices(&self) -> Result<()> {
        let binds = binds![self.device_id];
        self.conn
            .execute(CLEAR_ALL_SENDER_KEY_DEVICES, binds)
            .await?;
        Ok(())
    }

    // --- LID-PN mapping ---

    async fn get_lid_mapping(&self, lid: &str) -> Result<Option<LidPnMappingEntry>> {
        let binds = binds![lid, self.device_id];
        let row = self.conn.fetch_optional(GET_LID_MAPPING, binds).await?;
        Ok(row
            .map(|row| lid_mapping_row(&row))
            .transpose()?
            .map(protocol_rows::lid_entry))
    }

    async fn get_pn_mapping(&self, phone: &str) -> Result<Option<LidPnMappingEntry>> {
        let binds = binds![phone, self.device_id];
        let row = self.conn.fetch_optional(GET_PN_MAPPING, binds).await?;
        Ok(row
            .map(|row| lid_mapping_row(&row))
            .transpose()?
            .map(protocol_rows::lid_entry))
    }

    async fn put_lid_mapping(&self, entry: &LidPnMappingEntry) -> Result<()> {
        let binds = binds![
            entry.lid.as_str(),
            entry.phone_number.as_str(),
            entry.created_at,
            entry.learning_source.as_str(),
            entry.updated_at,
            self.device_id
        ];
        self.conn.execute(PUT_LID_MAPPING, binds).await?;
        Ok(())
    }

    async fn get_all_lid_mappings(&self) -> Result<Vec<LidPnMappingEntry>> {
        let binds = binds![self.device_id];
        let rows = self.conn.fetch_all(GET_ALL_LID_MAPPINGS, binds).await?;
        rows.iter()
            .map(|row| Ok(protocol_rows::lid_entry(lid_mapping_row(row)?)))
            .collect()
    }

    // --- Base key collision detection ---

    async fn save_base_key(&self, address: &str, message_id: &str, base_key: &[u8]) -> Result<()> {
        let row = joined_row(&[address.as_bytes(), message_id.as_bytes()]);
        let sealed = self.seal_at("base_keys", "base_key", &row, base_key)?;
        let binds = binds![address, message_id, sealed, self.device_id, now_secs()];
        self.conn.execute(SAVE_BASE_KEY, binds).await?;
        Ok(())
    }

    async fn has_same_base_key(
        &self,
        address: &str,
        message_id: &str,
        current_base_key: &[u8],
    ) -> Result<bool> {
        let binds = binds![address, message_id, self.device_id];
        let row = self.conn.fetch_optional(GET_BASE_KEY, binds).await?;
        // Open before comparing: the stored bytes are sealed, so a byte compare
        // against the plain key would never match.
        let row_key = joined_row(&[address.as_bytes(), message_id.as_bytes()]);
        let stored = first_blob(row)?
            .map(|stored| self.open_at("base_keys", "base_key", &row_key, &stored))
            .transpose()?;
        Ok(stored.as_deref() == Some(current_base_key))
    }

    async fn delete_base_key(&self, address: &str, message_id: &str) -> Result<()> {
        let binds = binds![address, message_id, self.device_id];
        self.conn.execute(DELETE_BASE_KEY, binds).await?;
        Ok(())
    }

    // --- Device registry ---

    async fn update_device_list(&self, record: DeviceListRecord) -> Result<()> {
        let devices_json = protocol_rows::devices_json(&record)?;
        let binds = binds![
            &*record.user,
            devices_json,
            record.timestamp,
            record.phash.as_deref(),
            self.device_id,
            now_secs(),
            record.raw_id.map(|r| r as i32)
        ];
        self.conn.execute(UPDATE_DEVICE_LIST, binds).await?;
        Ok(())
    }

    async fn get_devices(&self, user: &str) -> Result<Option<DeviceListRecord>> {
        let row = self
            .conn
            .fetch_optional(GET_DEVICES, binds![user, self.device_id])
            .await?;
        let decoded = row.map(|row| device_list_row(&row)).transpose()?;
        decoded.map(protocol_rows::device_list_record).transpose()
    }

    async fn delete_devices(&self, user: &str) -> Result<()> {
        let binds = binds![user, self.device_id];
        self.conn.execute(DELETE_DEVICES, binds).await?;
        Ok(())
    }

    // --- TcToken storage ---

    async fn get_tc_token(&self, jid: &str) -> Result<Option<TcTokenEntry>> {
        let binds = binds![jid, self.device_id];
        let row = self.conn.fetch_optional(GET_TC_TOKEN, binds).await?;
        let cipher = &self.cipher;
        row.map(|row| tc_token_entry(&row, 0, cipher, self.device_id, jid))
            .transpose()
    }

    async fn put_tc_token(&self, jid: &str, entry: &TcTokenEntry) -> Result<()> {
        let sealed = self.seal_at("tc_tokens", "token", jid.as_bytes(), &entry.token)?;
        let binds = binds![
            jid,
            sealed,
            entry.token_timestamp,
            entry.sender_timestamp,
            self.device_id,
            now_secs()
        ];
        self.conn.execute(PUT_TC_TOKEN, binds).await?;
        Ok(())
    }

    async fn delete_tc_token(&self, jid: &str) -> Result<()> {
        let binds = binds![jid, self.device_id];
        self.conn.execute(DELETE_TC_TOKEN, binds).await?;
        Ok(())
    }

    async fn get_all_tc_token_jids(&self) -> Result<Vec<String>> {
        let binds = binds![self.device_id];
        let rows = self.conn.fetch_all(GET_ALL_TC_TOKEN_JIDS, binds).await?;
        rows.iter().map(|row| text(row, 0)).collect()
    }

    /// Two independent windows, both of which must be stale before the row goes:
    /// the received token (expired or byte-empty) AND the sender bucket (expired
    /// or never set). Pruning on the token alone would drop recent sender state
    /// that the retry path still needs. Mirrors the reference `SqliteStore`.
    async fn delete_expired_tc_tokens(&self, token_cutoff: i64, sender_cutoff: i64) -> Result<u32> {
        let binds = binds![token_cutoff, sender_cutoff, self.device_id];
        let deleted = self.conn.execute(DELETE_EXPIRED_TC_TOKENS, binds).await?;
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
        let binds = binds![chat_jid, message_id, sealed, self.device_id, now_secs()];
        self.conn.execute(STORE_SENT_MESSAGE, binds).await?;
        Ok(())
    }

    async fn take_sent_message(&self, chat_jid: &str, message_id: &str) -> Result<Option<Vec<u8>>> {
        let tx = TursoTx::begin(&self.conn).await?;
        let binds = binds![chat_jid, message_id, self.device_id];
        let stored = first_blob(tx.fetch_optional(GET_SENT_MESSAGE, binds).await?)?;
        // Open before consuming: a blob that will not open must stay in its row.
        let payload = self.open_sent(chat_jid, message_id, stored)?;
        if payload.is_some() {
            let binds = binds![chat_jid, message_id, self.device_id];
            tx.execute(DELETE_SENT_MESSAGE, binds).await?;
        }
        tx.commit().await?;
        Ok(payload)
    }

    async fn delete_expired_sent_messages(&self, cutoff_timestamp: i64) -> Result<u32> {
        let binds = binds![cutoff_timestamp, self.device_id];
        let deleted = self
            .conn
            .execute(DELETE_EXPIRED_SENT_MESSAGES, binds)
            .await?;
        Ok(deleted as u32)
    }

    // --- Trait defaults overridden in #93 (docs/store-trait-defaults.md) ---

    async fn get_sent_message(&self, chat_jid: &str, message_id: &str) -> Result<Option<Vec<u8>>> {
        // Read-only on purpose: `take_sent_message` consumes, this must not.
        let binds = binds![chat_jid, message_id, self.device_id];
        let stored = first_blob(self.conn.fetch_optional(GET_SENT_MESSAGE, binds).await?)?;
        self.open_sent(chat_jid, message_id, stored)
    }

    async fn delete_expired_base_keys(&self, cutoff_timestamp: i64) -> Result<u32> {
        // The keepalive sweep calls this every cycle; the trait default was Ok(0),
        // so base_keys grew without bound.
        let binds = binds![cutoff_timestamp, self.device_id];
        let deleted = self.conn.execute(DELETE_EXPIRED_BASE_KEYS, binds).await?;
        Ok(deleted as u32)
    }

    async fn touch_tc_token_sender_timestamp(
        &self,
        jid: &str,
        sender_timestamp: i64,
    ) -> Result<()> {
        tc_token_sql::touch_sender_timestamp(&self.conn, self.device_id, jid, sender_timestamp)
            .await
    }

    async fn store_received_tc_token(
        &self,
        jid: &str,
        token: &[u8],
        token_timestamp: i64,
    ) -> Result<()> {
        tc_token_sql::store_received(
            &self.conn,
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
        protocol_batch_sql::put_lid_mappings(&self.conn, self.device_id, entries).await
    }

    async fn update_device_lists(&self, records: Vec<DeviceListRecord>) -> Result<()> {
        protocol_batch_sql::update_device_lists(&self.conn, self.device_id, records).await
    }

    async fn get_devices_batch(&self, users: &[&str]) -> Result<Vec<DeviceListRecord>> {
        protocol_batch_sql::get_devices_batch(&self.conn, self.device_id, users).await
    }

    async fn get_tc_tokens(&self, jids: &[String]) -> Result<Vec<Option<TcTokenEntry>>> {
        protocol_batch_sql::get_tc_tokens(&self.conn, &self.cipher, self.device_id, jids).await
    }
}
