//! `AppSyncStore` over SQLite. Mirrors `postgres/app_sync_store.rs` statement
//! for statement; only the dialect differs (`?` placeholders). Sync keys and
//! version state are protobuf blobs (`blob_codec`, #31); MACs are raw bytes.

use async_trait::async_trait;
use wacore::appstate::hash::HashState;
use wacore::appstate::processor::AppStateMutationMAC;
use wacore::store::error::Result;
use wacore::store::traits::{AppStateSyncKey, AppSyncStore};

use super::SqliteBackend;
use crate::storage::blob_codec::{
    decode_app_state_sync_key, decode_hash_state, encode_app_state_sync_key, encode_hash_state,
};
use crate::storage::sqlx_error::db;

#[async_trait]
impl AppSyncStore for SqliteBackend {
    async fn get_sync_key(&self, key_id: &[u8]) -> Result<Option<AppStateSyncKey>> {
        let row: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT key_data FROM app_state_keys WHERE key_id = ? AND device_id = ?",
        )
        .bind(key_id)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        match row {
            None => Ok(None),
            Some(bytes) => Ok(Some(decode_app_state_sync_key(&bytes)?)),
        }
    }

    async fn set_sync_key(&self, key_id: &[u8], key: AppStateSyncKey) -> Result<()> {
        let data = encode_app_state_sync_key(&key);
        sqlx::query(
            "INSERT INTO app_state_keys (key_id, key_data, device_id) VALUES (?, ?, ?)
             ON CONFLICT (key_id, device_id) DO UPDATE SET key_data = EXCLUDED.key_data",
        )
        .bind(key_id)
        .bind(&data)
        .bind(self.device_id)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    // `main` made absence meaningful: no row means "never synced" (the lib
    // bootstraps from a snapshot), while a row at version 0 means a collection
    // that synced and is legitimately empty (the lib asks for patches).
    // Collapsing both into `HashState::default()` made an empty collection
    // re-request a snapshot forever, so a missing row now reads as `None`.
    async fn get_version(&self, name: &str) -> Result<Option<HashState>> {
        let row: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT state_data FROM app_state_versions WHERE name = ? AND device_id = ?",
        )
        .bind(name)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        match row {
            None => Ok(None),
            Some(bytes) => Ok(Some(decode_hash_state(&bytes)?)),
        }
    }

    /// Forget a collection's version, returning it to the never-synced state.
    /// A missing row is a no-op, not an error; only this device's row is
    /// touched.
    async fn delete_version(&self, name: &str) -> Result<()> {
        sqlx::query("DELETE FROM app_state_versions WHERE name = ? AND device_id = ?")
            .bind(name)
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    async fn set_version(&self, name: &str, state: HashState) -> Result<()> {
        let data = encode_hash_state(&state);
        sqlx::query(
            "INSERT INTO app_state_versions (name, state_data, device_id) VALUES (?, ?, ?)
             ON CONFLICT (name, device_id) DO UPDATE SET state_data = EXCLUDED.state_data",
        )
        .bind(name)
        .bind(&data)
        .bind(self.device_id)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn put_mutation_macs(
        &self,
        name: &str,
        version: u64,
        mutations: &[AppStateMutationMAC],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        for m in mutations {
            sqlx::query(
                "INSERT INTO app_state_mutation_macs (name, version, index_mac, value_mac, device_id)
                 VALUES (?, ?, ?, ?, ?)
                 ON CONFLICT (name, index_mac, device_id)
                 DO UPDATE SET version = EXCLUDED.version, value_mac = EXCLUDED.value_mac",
            )
            .bind(name)
            .bind(version as i64)
            .bind(m.index_mac.as_slice())
            .bind(m.value_mac.as_slice())
            .bind(self.device_id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        }
        tx.commit().await.map_err(db)?;
        Ok(())
    }

    async fn get_mutation_mac(&self, name: &str, index_mac: &[u8]) -> Result<Option<Vec<u8>>> {
        let row: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT value_mac FROM app_state_mutation_macs
             WHERE name = ? AND index_mac = ? AND device_id = ?",
        )
        .bind(name)
        .bind(index_mac)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row)
    }

    async fn delete_mutation_macs(&self, name: &str, index_macs: &[Vec<u8>]) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        for index_mac in index_macs {
            sqlx::query(
                "DELETE FROM app_state_mutation_macs
                 WHERE name = ? AND index_mac = ? AND device_id = ?",
            )
            .bind(name)
            .bind(index_mac.as_slice())
            .bind(self.device_id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        }
        tx.commit().await.map_err(db)?;
        Ok(())
    }

    /// Wipe a collection's MAC store. The lib calls this on snapshot re-sync:
    /// the snapshot rebuilds the ltHash from scratch, so a MAC left over from
    /// the pre-snapshot timeline would corrupt the next patch's ltHash.
    async fn clear_mutation_macs(&self, name: &str) -> Result<()> {
        sqlx::query("DELETE FROM app_state_mutation_macs WHERE name = ? AND device_id = ?")
            .bind(name)
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    async fn get_latest_sync_key_id(&self) -> Result<Option<Vec<u8>>> {
        let row: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT key_id FROM app_state_keys WHERE device_id = ? ORDER BY key_id DESC LIMIT 1",
        )
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row)
    }
}
