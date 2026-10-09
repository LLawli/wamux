//! `SignalStore` for the turso family, method for method the sql family's
//! (#106). All key material is stored as raw bytes, no transform.

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use wacore::store::error::{Result, StoreError};
use wacore::store::traits::SignalStore;

use super::exec::binds;
use super::row_values::{blob, first_blob, int, int32};
use super::{TursoBackend, signal_sql};
use crate::storage::statements::signal::{
    DELETE_IDENTITY, DELETE_PREKEY, DELETE_SENDER_KEY, DELETE_SESSION, GET_SENDER_KEY, GET_SESSION,
    LOAD_ALL_SIGNED_PREKEYS, LOAD_IDENTITY, LOAD_PREKEY, LOAD_SIGNED_PREKEY, MAX_PREKEY_ID,
    PUT_IDENTITY, PUT_SENDER_KEY, PUT_SESSION, REMOVE_SIGNED_PREKEY, STORE_PREKEY,
    STORE_SIGNED_PREKEY,
};

#[async_trait]
impl SignalStore for TursoBackend {
    // --- Identities ---

    async fn put_identity(&self, address: &str, key: [u8; 32]) -> Result<()> {
        let sealed = self.seal_at("identities", "key", address.as_bytes(), &key)?;
        let binds = binds![address, sealed, self.device_id];
        self.conn.execute(PUT_IDENTITY, binds).await?;
        Ok(())
    }

    async fn load_identity(&self, address: &str) -> Result<Option<[u8; 32]>> {
        let binds = binds![address, self.device_id];
        match first_blob(self.conn.fetch_optional(LOAD_IDENTITY, binds).await?)? {
            None => Ok(None),
            Some(stored) => {
                let bytes = self.open_at("identities", "key", address.as_bytes(), &stored)?;
                let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
                    StoreError::Validation(format!("Invalid identity key length: {}", bytes.len()))
                })?;
                Ok(Some(arr))
            }
        }
    }

    async fn delete_identity(&self, address: &str) -> Result<()> {
        let binds = binds![address, self.device_id];
        self.conn.execute(DELETE_IDENTITY, binds).await?;
        Ok(())
    }

    // --- Sessions ---

    async fn get_session(&self, address: &str) -> Result<Option<Bytes>> {
        let binds = binds![address, self.device_id];
        let row = self.conn.fetch_optional(GET_SESSION, binds).await?;
        let stored = first_blob(row)?;
        let record = stored
            .map(|stored| self.open_at("sessions", "record", address.as_bytes(), &stored))
            .transpose()?;
        Ok(record.map(Bytes::from))
    }

    async fn put_session(&self, address: &str, session: &[u8]) -> Result<()> {
        let sealed = self.seal_at("sessions", "record", address.as_bytes(), session)?;
        let binds = binds![address, sealed, self.device_id];
        self.conn.execute(PUT_SESSION, binds).await?;
        Ok(())
    }

    async fn delete_session(&self, address: &str) -> Result<()> {
        let binds = binds![address, self.device_id];
        self.conn.execute(DELETE_SESSION, binds).await?;
        Ok(())
    }

    // --- PreKeys ---

    async fn store_prekey(&self, id: u32, record: &[u8], uploaded: bool) -> Result<()> {
        let sealed = self.seal_at("prekeys", "key", &id.to_be_bytes(), record)?;
        let binds = binds![id as i32, sealed, uploaded, self.device_id];
        self.conn.execute(STORE_PREKEY, binds).await?;
        Ok(())
    }

    /// UPDATE, never upsert. A prekey consumed (and deleted) between the upload
    /// snapshot and this call has to stay deleted; an upsert would resurrect it
    /// with an empty record and hand the server a key we can no longer answer.
    /// One UPDATE per id in one transaction, like SQLite (`signal_sql`).
    async fn mark_prekeys_uploaded(&self, ids: &[u32]) -> Result<()> {
        signal_sql::mark_uploaded(&self.conn, self.device_id, ids).await
    }

    async fn load_prekey(&self, id: u32) -> Result<Option<Bytes>> {
        let binds = binds![id as i32, self.device_id];
        let row = self.conn.fetch_optional(LOAD_PREKEY, binds).await?;
        let record = first_blob(row)?
            .map(|stored| self.open_at("prekeys", "key", &id.to_be_bytes(), &stored))
            .transpose()?;
        Ok(record.map(Bytes::from))
    }

    async fn remove_prekey(&self, id: u32) -> Result<()> {
        let binds = binds![id as i32, self.device_id];
        self.conn.execute(DELETE_PREKEY, binds).await?;
        Ok(())
    }

    async fn get_max_prekey_id(&self) -> Result<u32> {
        let row = self
            .conn
            .fetch_optional(MAX_PREKEY_ID, binds![self.device_id])
            .await?;
        let max = row.map(|row| int(&row, 0)).transpose()?.unwrap_or_default();
        Ok(max as u32)
    }

    // --- Signed PreKeys ---

    async fn store_signed_prekey(&self, id: u32, record: &[u8]) -> Result<()> {
        let sealed = self.seal_at("signed_prekeys", "record", &id.to_be_bytes(), record)?;
        let binds = binds![id as i32, sealed, self.device_id];
        self.conn.execute(STORE_SIGNED_PREKEY, binds).await?;
        Ok(())
    }

    async fn load_signed_prekey(&self, id: u32) -> Result<Option<Vec<u8>>> {
        let binds = binds![id as i32, self.device_id];
        let row = self.conn.fetch_optional(LOAD_SIGNED_PREKEY, binds).await?;
        first_blob(row)?
            .map(|stored| self.open_at("signed_prekeys", "record", &id.to_be_bytes(), &stored))
            .transpose()
    }

    async fn load_all_signed_prekeys(&self) -> Result<Vec<(u32, Vec<u8>)>> {
        let binds = binds![self.device_id];
        let rows = self.conn.fetch_all(LOAD_ALL_SIGNED_PREKEYS, binds).await?;
        rows.iter()
            .map(|row| {
                let id = int32(row, 0)?;
                let stored = blob(row, 1)?;
                let record =
                    self.open_at("signed_prekeys", "record", &id.to_be_bytes(), &stored)?;
                Ok((id as u32, record))
            })
            .collect()
    }

    async fn remove_signed_prekey(&self, id: u32) -> Result<()> {
        let binds = binds![id as i32, self.device_id];
        self.conn.execute(REMOVE_SIGNED_PREKEY, binds).await?;
        Ok(())
    }

    // --- Sender Keys ---

    async fn put_sender_key(&self, address: &str, record: &[u8]) -> Result<()> {
        let sealed = self.seal_at("sender_keys", "record", address.as_bytes(), record)?;
        let binds = binds![address, sealed, self.device_id];
        self.conn.execute(PUT_SENDER_KEY, binds).await?;
        Ok(())
    }

    async fn get_sender_key(&self, address: &str) -> Result<Option<Vec<u8>>> {
        let binds = binds![address, self.device_id];
        let row = self.conn.fetch_optional(GET_SENDER_KEY, binds).await?;
        first_blob(row)?
            .map(|stored| self.open_at("sender_keys", "record", address.as_bytes(), &stored))
            .transpose()
    }

    async fn delete_sender_key(&self, address: &str) -> Result<()> {
        let binds = binds![address, self.device_id];
        self.conn.execute(DELETE_SENDER_KEY, binds).await?;
        Ok(())
    }

    // --- Batches (#104): one transaction or one query per chunk, see `signal_sql` ---

    async fn put_identities_batch(&self, identities: &[(Arc<str>, [u8; 32])]) -> Result<()> {
        signal_sql::put_identities(&self.conn, &self.cipher, self.device_id, identities).await
    }

    async fn delete_identities_batch(&self, addresses: &[Arc<str>]) -> Result<()> {
        signal_sql::delete_identities(&self.conn, self.device_id, addresses).await
    }

    async fn put_sessions_batch(&self, sessions: &[(Arc<str>, Bytes)]) -> Result<()> {
        signal_sql::put_sessions(&self.conn, &self.cipher, self.device_id, sessions).await
    }

    async fn get_sessions_batch(&self, addresses: &[Arc<str>]) -> Result<Vec<(Arc<str>, Bytes)>> {
        signal_sql::get_sessions(&self.conn, &self.cipher, self.device_id, addresses).await
    }

    async fn delete_sessions_batch(&self, addresses: &[Arc<str>]) -> Result<()> {
        signal_sql::delete_sessions(&self.conn, self.device_id, addresses).await
    }

    async fn store_prekeys_batch(&self, keys: &[(u32, Bytes)], uploaded: bool) -> Result<()> {
        signal_sql::store_prekeys(&self.conn, &self.cipher, self.device_id, keys, uploaded).await
    }

    async fn load_prekeys_batch(&self, ids: &[u32]) -> Result<Vec<(u32, Bytes)>> {
        signal_sql::load_prekeys(&self.conn, &self.cipher, self.device_id, ids).await
    }

    async fn remove_prekeys_batch(&self, ids: &[u32]) -> Result<()> {
        signal_sql::remove_prekeys(&self.conn, self.device_id, ids).await
    }

    async fn put_sender_keys_batch(&self, sender_keys: &[(Arc<str>, Bytes)]) -> Result<()> {
        signal_sql::put_sender_keys(&self.conn, &self.cipher, self.device_id, sender_keys).await
    }

    async fn delete_sender_keys_batch(&self, addresses: &[Arc<str>]) -> Result<()> {
        signal_sql::delete_sender_keys(&self.conn, self.device_id, addresses).await
    }
}
