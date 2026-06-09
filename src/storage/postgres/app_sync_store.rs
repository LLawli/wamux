//! `AppSyncStore` over Postgres. Sync keys and version state are bincode-standard
//! blobs (matching the reference); MACs are raw bytes.

use async_trait::async_trait;
use wacore::appstate::hash::HashState;
use wacore::appstate::processor::AppStateMutationMAC;
use wacore::store::error::Result;
use wacore::store::traits::{AppStateSyncKey, AppSyncStore};

use super::error_map::db;
use super::{PgBackend, bincode_decode, bincode_encode};

#[async_trait]
impl AppSyncStore for PgBackend {
    async fn get_sync_key(&self, key_id: &[u8]) -> Result<Option<AppStateSyncKey>> {
        let row: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT key_data FROM app_state_keys WHERE key_id = $1 AND device_id = $2",
        )
        .bind(key_id)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        match row {
            None => Ok(None),
            Some(bytes) => Ok(Some(bincode_decode(&bytes)?)),
        }
    }

    async fn set_sync_key(&self, key_id: &[u8], key: AppStateSyncKey) -> Result<()> {
        let data = bincode_encode(&key)?;
        sqlx::query(
            "INSERT INTO app_state_keys (key_id, key_data, device_id) VALUES ($1, $2, $3)
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

    async fn get_version(&self, name: &str) -> Result<HashState> {
        let row: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT state_data FROM app_state_versions WHERE name = $1 AND device_id = $2",
        )
        .bind(name)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        match row {
            None => Ok(HashState::default()),
            Some(bytes) => bincode_decode(&bytes),
        }
    }

    async fn set_version(&self, name: &str, state: HashState) -> Result<()> {
        let data = bincode_encode(&state)?;
        sqlx::query(
            "INSERT INTO app_state_versions (name, state_data, device_id) VALUES ($1, $2, $3)
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
                 VALUES ($1, $2, $3, $4, $5)
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
             WHERE name = $1 AND index_mac = $2 AND device_id = $3",
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
                 WHERE name = $1 AND index_mac = $2 AND device_id = $3",
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

    async fn get_latest_sync_key_id(&self) -> Result<Option<Vec<u8>>> {
        let row: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT key_id FROM app_state_keys WHERE device_id = $1 ORDER BY key_id DESC LIMIT 1",
        )
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row)
    }
}
