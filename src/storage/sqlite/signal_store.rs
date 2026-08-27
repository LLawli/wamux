//! `SignalStore` over SQLite. Mirrors `postgres/signal_store.rs` statement for
//! statement; only the dialect differs (`?` placeholders). All key material is
//! stored as raw bytes, matching the reference (no transform).

use async_trait::async_trait;
use bytes::Bytes;
use wacore::store::error::{Result, StoreError};
use wacore::store::traits::SignalStore;

use super::SqliteBackend;
use crate::storage::sqlx_error::db;

#[async_trait]
impl SignalStore for SqliteBackend {
    // --- Identities ---

    async fn put_identity(&self, address: &str, key: [u8; 32]) -> Result<()> {
        sqlx::query(
            "INSERT INTO identities (address, key, device_id) VALUES (?, ?, ?)
             ON CONFLICT (address, device_id) DO UPDATE SET key = EXCLUDED.key",
        )
        .bind(address)
        .bind(&key[..])
        .bind(self.device_id)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn load_identity(&self, address: &str) -> Result<Option<[u8; 32]>> {
        let row: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT key FROM identities WHERE address = ? AND device_id = ?")
                .bind(address)
                .bind(self.device_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(db)?;
        match row {
            None => Ok(None),
            Some(bytes) => {
                let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
                    StoreError::Validation(format!("Invalid identity key length: {}", bytes.len()))
                })?;
                Ok(Some(arr))
            }
        }
    }

    async fn delete_identity(&self, address: &str) -> Result<()> {
        sqlx::query("DELETE FROM identities WHERE address = ? AND device_id = ?")
            .bind(address)
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    // --- Sessions ---

    async fn get_session(&self, address: &str) -> Result<Option<Bytes>> {
        let row: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT record FROM sessions WHERE address = ? AND device_id = ?")
                .bind(address)
                .bind(self.device_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(db)?;
        Ok(row.map(Bytes::from))
    }

    async fn put_session(&self, address: &str, session: &[u8]) -> Result<()> {
        sqlx::query(
            "INSERT INTO sessions (address, record, device_id) VALUES (?, ?, ?)
             ON CONFLICT (address, device_id) DO UPDATE SET record = EXCLUDED.record",
        )
        .bind(address)
        .bind(session)
        .bind(self.device_id)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn delete_session(&self, address: &str) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE address = ? AND device_id = ?")
            .bind(address)
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    // --- PreKeys ---

    async fn store_prekey(&self, id: u32, record: &[u8], uploaded: bool) -> Result<()> {
        sqlx::query(
            "INSERT INTO prekeys (id, key, uploaded, device_id) VALUES (?, ?, ?, ?)
             ON CONFLICT (id, device_id) DO UPDATE SET key = EXCLUDED.key, uploaded = EXCLUDED.uploaded",
        )
        .bind(id as i32)
        .bind(record)
        .bind(uploaded)
        .bind(self.device_id)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn load_prekey(&self, id: u32) -> Result<Option<Bytes>> {
        let row: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT key FROM prekeys WHERE id = ? AND device_id = ?")
                .bind(id as i32)
                .bind(self.device_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(db)?;
        Ok(row.map(Bytes::from))
    }

    async fn remove_prekey(&self, id: u32) -> Result<()> {
        sqlx::query("DELETE FROM prekeys WHERE id = ? AND device_id = ?")
            .bind(id as i32)
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    async fn get_max_prekey_id(&self) -> Result<u32> {
        let max: i32 =
            sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) FROM prekeys WHERE device_id = ?")
                .bind(self.device_id)
                .fetch_one(&self.pool)
                .await
                .map_err(db)?;
        Ok(max as u32)
    }

    // --- Signed PreKeys ---

    async fn store_signed_prekey(&self, id: u32, record: &[u8]) -> Result<()> {
        sqlx::query(
            "INSERT INTO signed_prekeys (id, record, device_id) VALUES (?, ?, ?)
             ON CONFLICT (id, device_id) DO UPDATE SET record = EXCLUDED.record",
        )
        .bind(id as i32)
        .bind(record)
        .bind(self.device_id)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn load_signed_prekey(&self, id: u32) -> Result<Option<Vec<u8>>> {
        let row: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT record FROM signed_prekeys WHERE id = ? AND device_id = ?")
                .bind(id as i32)
                .bind(self.device_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(db)?;
        Ok(row)
    }

    async fn load_all_signed_prekeys(&self) -> Result<Vec<(u32, Vec<u8>)>> {
        let rows: Vec<(i32, Vec<u8>)> =
            sqlx::query_as("SELECT id, record FROM signed_prekeys WHERE device_id = ?")
                .bind(self.device_id)
                .fetch_all(&self.pool)
                .await
                .map_err(db)?;
        Ok(rows.into_iter().map(|(id, rec)| (id as u32, rec)).collect())
    }

    async fn remove_signed_prekey(&self, id: u32) -> Result<()> {
        sqlx::query("DELETE FROM signed_prekeys WHERE id = ? AND device_id = ?")
            .bind(id as i32)
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    // --- Sender Keys ---

    async fn put_sender_key(&self, address: &str, record: &[u8]) -> Result<()> {
        sqlx::query(
            "INSERT INTO sender_keys (address, record, device_id) VALUES (?, ?, ?)
             ON CONFLICT (address, device_id) DO UPDATE SET record = EXCLUDED.record",
        )
        .bind(address)
        .bind(record)
        .bind(self.device_id)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn get_sender_key(&self, address: &str) -> Result<Option<Vec<u8>>> {
        let row: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT record FROM sender_keys WHERE address = ? AND device_id = ?",
        )
        .bind(address)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row)
    }

    async fn delete_sender_key(&self, address: &str) -> Result<()> {
        sqlx::query("DELETE FROM sender_keys WHERE address = ? AND device_id = ?")
            .bind(address)
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }
}
