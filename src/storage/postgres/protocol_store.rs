//! `ProtocolStore` over Postgres: per-device sender-key tracking, LID-PN
//! mappings, base keys, device registry, tc tokens, and the sent-message cache.

use async_trait::async_trait;
use wacore::store::error::Result;
use wacore::store::traits::{
    DeviceInfo, DeviceListRecord, LidPnMappingEntry, ProtocolStore, TcTokenEntry,
};

use super::{PgBackend, now_secs};
use crate::storage::sqlx_error::db;

#[async_trait]
impl ProtocolStore for PgBackend {
    // --- Per-device sender key tracking ---

    async fn get_sender_key_devices(&self, group_jid: &str) -> Result<Vec<(String, bool)>> {
        let rows: Vec<(String, i32)> = sqlx::query_as(
            "SELECT device_jid, has_key FROM sender_key_devices WHERE group_jid = $1 AND device_id = $2",
        )
        .bind(group_jid)
        .bind(self.device_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(rows.into_iter().map(|(jid, has)| (jid, has != 0)).collect())
    }

    async fn set_sender_key_status(&self, group_jid: &str, entries: &[(&str, bool)]) -> Result<()> {
        let now = now_secs();
        let mut tx = self.pool.begin().await.map_err(db)?;
        for (device_jid, has_key) in entries {
            sqlx::query(
                "INSERT INTO sender_key_devices (group_jid, device_jid, has_key, device_id, updated_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (group_jid, device_jid, device_id)
                 DO UPDATE SET has_key = EXCLUDED.has_key, updated_at = EXCLUDED.updated_at",
            )
            .bind(group_jid)
            .bind(device_jid)
            .bind(i32::from(*has_key))
            .bind(self.device_id)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        }
        tx.commit().await.map_err(db)?;
        Ok(())
    }

    async fn clear_sender_key_devices(&self, group_jid: &str) -> Result<()> {
        sqlx::query("DELETE FROM sender_key_devices WHERE group_jid = $1 AND device_id = $2")
            .bind(group_jid)
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    async fn delete_sender_key_device_rows(&self, device_jids: &[&str]) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        for device_jid in device_jids {
            sqlx::query("DELETE FROM sender_key_devices WHERE device_jid = $1 AND device_id = $2")
                .bind(device_jid)
                .bind(self.device_id)
                .execute(&mut *tx)
                .await
                .map_err(db)?;
        }
        tx.commit().await.map_err(db)?;
        Ok(())
    }

    async fn clear_all_sender_key_devices(&self) -> Result<()> {
        sqlx::query("DELETE FROM sender_key_devices WHERE device_id = $1")
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    // --- LID-PN mapping ---

    async fn get_lid_mapping(&self, lid: &str) -> Result<Option<LidPnMappingEntry>> {
        let row: Option<(String, String, i64, String, i64)> = sqlx::query_as(
            "SELECT lid, phone_number, created_at, learning_source, updated_at
             FROM lid_pn_mapping WHERE lid = $1 AND device_id = $2",
        )
        .bind(lid)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row.map(
            |(lid, phone_number, created_at, learning_source, updated_at)| LidPnMappingEntry {
                lid,
                phone_number,
                created_at,
                updated_at,
                learning_source,
            },
        ))
    }

    async fn get_pn_mapping(&self, phone: &str) -> Result<Option<LidPnMappingEntry>> {
        let row: Option<(String, String, i64, String, i64)> = sqlx::query_as(
            "SELECT lid, phone_number, created_at, learning_source, updated_at
             FROM lid_pn_mapping WHERE phone_number = $1 AND device_id = $2
             ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(phone)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row.map(
            |(lid, phone_number, created_at, learning_source, updated_at)| LidPnMappingEntry {
                lid,
                phone_number,
                created_at,
                updated_at,
                learning_source,
            },
        ))
    }

    async fn put_lid_mapping(&self, entry: &LidPnMappingEntry) -> Result<()> {
        sqlx::query(
            "INSERT INTO lid_pn_mapping
                (lid, phone_number, created_at, learning_source, updated_at, device_id)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (lid, device_id) DO UPDATE SET
                phone_number = EXCLUDED.phone_number,
                learning_source = EXCLUDED.learning_source,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(&entry.lid)
        .bind(&entry.phone_number)
        .bind(entry.created_at)
        .bind(&entry.learning_source)
        .bind(entry.updated_at)
        .bind(self.device_id)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn get_all_lid_mappings(&self) -> Result<Vec<LidPnMappingEntry>> {
        let rows: Vec<(String, String, i64, String, i64)> = sqlx::query_as(
            "SELECT lid, phone_number, created_at, learning_source, updated_at
             FROM lid_pn_mapping WHERE device_id = $1",
        )
        .bind(self.device_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(rows
            .into_iter()
            .map(
                |(lid, phone_number, created_at, learning_source, updated_at)| LidPnMappingEntry {
                    lid,
                    phone_number,
                    created_at,
                    updated_at,
                    learning_source,
                },
            )
            .collect())
    }

    // --- Base key collision detection ---

    async fn save_base_key(&self, address: &str, message_id: &str, base_key: &[u8]) -> Result<()> {
        sqlx::query(
            "INSERT INTO base_keys (address, message_id, base_key, device_id, created_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (address, message_id, device_id) DO UPDATE SET base_key = EXCLUDED.base_key",
        )
        .bind(address)
        .bind(message_id)
        .bind(base_key)
        .bind(self.device_id)
        .bind(now_secs())
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn has_same_base_key(
        &self,
        address: &str,
        message_id: &str,
        current_base_key: &[u8],
    ) -> Result<bool> {
        let row: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT base_key FROM base_keys
             WHERE address = $1 AND message_id = $2 AND device_id = $3",
        )
        .bind(address)
        .bind(message_id)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row.as_deref() == Some(current_base_key))
    }

    async fn delete_base_key(&self, address: &str, message_id: &str) -> Result<()> {
        sqlx::query(
            "DELETE FROM base_keys WHERE address = $1 AND message_id = $2 AND device_id = $3",
        )
        .bind(address)
        .bind(message_id)
        .bind(self.device_id)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    // --- Device registry ---

    async fn update_device_list(&self, record: DeviceListRecord) -> Result<()> {
        let devices_json = serde_json::to_string(&record.devices)
            .map_err(|e| wacore::store::error::StoreError::Serialization(Box::new(e)))?;
        sqlx::query(
            "INSERT INTO device_registry
                (user_id, devices_json, timestamp, phash, device_id, updated_at, raw_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (user_id, device_id) DO UPDATE SET
                devices_json = EXCLUDED.devices_json,
                timestamp = EXCLUDED.timestamp,
                phash = EXCLUDED.phash,
                updated_at = EXCLUDED.updated_at,
                raw_id = EXCLUDED.raw_id",
        )
        .bind(&record.user)
        .bind(&devices_json)
        .bind(record.timestamp)
        .bind(record.phash.as_deref())
        .bind(self.device_id)
        .bind(now_secs())
        .bind(record.raw_id.map(|r| r as i32))
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn get_devices(&self, user: &str) -> Result<Option<DeviceListRecord>> {
        let row: Option<(String, String, i64, Option<String>, Option<i32>)> = sqlx::query_as(
            "SELECT user_id, devices_json, timestamp, phash, raw_id
             FROM device_registry WHERE user_id = $1 AND device_id = $2",
        )
        .bind(user)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        match row {
            None => Ok(None),
            Some((user, devices_json, timestamp, phash, raw_id)) => {
                let devices: Vec<DeviceInfo> = serde_json::from_str(&devices_json)
                    .map_err(|e| wacore::store::error::StoreError::Serialization(Box::new(e)))?;
                Ok(Some(DeviceListRecord {
                    user,
                    devices,
                    timestamp,
                    phash,
                    raw_id: raw_id.map(|r| r as u32),
                }))
            }
        }
    }

    async fn delete_devices(&self, user: &str) -> Result<()> {
        sqlx::query("DELETE FROM device_registry WHERE user_id = $1 AND device_id = $2")
            .bind(user)
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    // --- TcToken storage ---

    async fn get_tc_token(&self, jid: &str) -> Result<Option<TcTokenEntry>> {
        let row: Option<(Vec<u8>, i64, Option<i64>)> = sqlx::query_as(
            "SELECT token, token_timestamp, sender_timestamp
             FROM tc_tokens WHERE jid = $1 AND device_id = $2",
        )
        .bind(jid)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(
            row.map(|(token, token_timestamp, sender_timestamp)| TcTokenEntry {
                token,
                token_timestamp,
                sender_timestamp,
            }),
        )
    }

    async fn put_tc_token(&self, jid: &str, entry: &TcTokenEntry) -> Result<()> {
        sqlx::query(
            "INSERT INTO tc_tokens
                (jid, token, token_timestamp, sender_timestamp, device_id, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (jid, device_id) DO UPDATE SET
                token = EXCLUDED.token,
                token_timestamp = EXCLUDED.token_timestamp,
                sender_timestamp = EXCLUDED.sender_timestamp,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(jid)
        .bind(entry.token.as_slice())
        .bind(entry.token_timestamp)
        .bind(entry.sender_timestamp)
        .bind(self.device_id)
        .bind(now_secs())
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn delete_tc_token(&self, jid: &str) -> Result<()> {
        sqlx::query("DELETE FROM tc_tokens WHERE jid = $1 AND device_id = $2")
            .bind(jid)
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    async fn get_all_tc_token_jids(&self) -> Result<Vec<String>> {
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT jid FROM tc_tokens WHERE device_id = $1")
                .bind(self.device_id)
                .fetch_all(&self.pool)
                .await
                .map_err(db)?;
        Ok(rows)
    }

    async fn delete_expired_tc_tokens(&self, cutoff_timestamp: i64) -> Result<u32> {
        let res =
            sqlx::query("DELETE FROM tc_tokens WHERE token_timestamp < $1 AND device_id = $2")
                .bind(cutoff_timestamp)
                .bind(self.device_id)
                .execute(&self.pool)
                .await
                .map_err(db)?;
        Ok(res.rows_affected() as u32)
    }

    // --- Sent message cache (retry support) ---

    async fn store_sent_message(
        &self,
        chat_jid: &str,
        message_id: &str,
        payload: &[u8],
    ) -> Result<()> {
        // REPLACE semantics in the reference reset created_at; mirror that.
        sqlx::query(
            "INSERT INTO sent_messages (chat_jid, message_id, payload, device_id, created_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (chat_jid, message_id, device_id)
             DO UPDATE SET payload = EXCLUDED.payload, created_at = EXCLUDED.created_at",
        )
        .bind(chat_jid)
        .bind(message_id)
        .bind(payload)
        .bind(self.device_id)
        .bind(now_secs())
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn take_sent_message(&self, chat_jid: &str, message_id: &str) -> Result<Option<Vec<u8>>> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let payload: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT payload FROM sent_messages
             WHERE chat_jid = $1 AND message_id = $2 AND device_id = $3",
        )
        .bind(chat_jid)
        .bind(message_id)
        .bind(self.device_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        if payload.is_some() {
            sqlx::query(
                "DELETE FROM sent_messages
                 WHERE chat_jid = $1 AND message_id = $2 AND device_id = $3",
            )
            .bind(chat_jid)
            .bind(message_id)
            .bind(self.device_id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        }
        tx.commit().await.map_err(db)?;
        Ok(payload)
    }

    async fn delete_expired_sent_messages(&self, cutoff_timestamp: i64) -> Result<u32> {
        let res = sqlx::query("DELETE FROM sent_messages WHERE created_at < $1 AND device_id = $2")
            .bind(cutoff_timestamp)
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(res.rows_affected() as u32)
    }
}
