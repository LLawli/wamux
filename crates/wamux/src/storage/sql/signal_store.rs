//! `SignalStore` for the SQL family. All key material is stored as raw bytes,
//! matching the whatsapp-rust sqlite reference (no transform).

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use wacore::store::error::{Result, StoreError};
use wacore::store::traits::SignalStore;

use super::signal_sql;
use super::{SqlBackend, prekeys_sql};
use crate::storage::statements::signal::{
    DELETE_IDENTITY, DELETE_PREKEY, DELETE_SENDER_KEY, DELETE_SESSION, GET_SENDER_KEY, GET_SESSION,
    LOAD_ALL_SIGNED_PREKEYS, LOAD_IDENTITY, LOAD_PREKEY, LOAD_SIGNED_PREKEY, MAX_PREKEY_ID,
    PUT_IDENTITY, PUT_SENDER_KEY, PUT_SESSION, REMOVE_SIGNED_PREKEY, STORE_PREKEY,
    STORE_SIGNED_PREKEY,
};

#[async_trait]
impl SignalStore for SqlBackend {
    // --- Identities ---

    async fn put_identity(&self, address: &str, key: [u8; 32]) -> Result<()> {
        let sealed = self.seal_at("identities", "key", address.as_bytes(), &key)?;
        execute_sql!(
            &self.pool,
            PUT_IDENTITY,
            address,
            &sealed[..],
            self.device_id
        )?;
        Ok(())
    }

    async fn load_identity(&self, address: &str) -> Result<Option<[u8; 32]>> {
        let row =
            scalar_optional_sql!(Vec<u8>, &self.pool, LOAD_IDENTITY, address, self.device_id)?;
        match row {
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
        execute_sql!(&self.pool, DELETE_IDENTITY, address, self.device_id)?;
        Ok(())
    }

    // --- Sessions ---

    async fn get_session(&self, address: &str) -> Result<Option<Bytes>> {
        let row = scalar_optional_sql!(Vec<u8>, &self.pool, GET_SESSION, address, self.device_id)?;
        let row = row
            .map(|stored| self.open_at("sessions", "record", address.as_bytes(), &stored))
            .transpose()?;
        Ok(row.map(Bytes::from))
    }

    async fn put_session(&self, address: &str, session: &[u8]) -> Result<()> {
        let sealed = self.seal_at("sessions", "record", address.as_bytes(), session)?;
        execute_sql!(
            &self.pool,
            PUT_SESSION,
            address,
            &sealed[..],
            self.device_id
        )?;
        Ok(())
    }

    async fn delete_session(&self, address: &str) -> Result<()> {
        execute_sql!(&self.pool, DELETE_SESSION, address, self.device_id)?;
        Ok(())
    }

    // --- PreKeys ---

    async fn store_prekey(&self, id: u32, record: &[u8], uploaded: bool) -> Result<()> {
        let sealed = self.seal_at("prekeys", "key", &id.to_be_bytes(), record)?;
        execute_sql!(
            &self.pool,
            STORE_PREKEY,
            id as i32,
            &sealed[..],
            uploaded,
            self.device_id
        )?;
        Ok(())
    }

    /// UPDATE, never upsert. A prekey consumed (and deleted) between the upload
    /// snapshot and this call has to stay deleted; an upsert would resurrect it
    /// with an empty record and hand the server a key we can no longer answer.
    /// One of the few statements written per driver (`prekeys_sql`): Postgres
    /// binds an array, SQLite has no array bind.
    async fn mark_prekeys_uploaded(&self, ids: &[u32]) -> Result<()> {
        prekeys_sql::mark_uploaded(&self.pool, self.device_id, ids).await
    }

    async fn load_prekey(&self, id: u32) -> Result<Option<Bytes>> {
        let row =
            scalar_optional_sql!(Vec<u8>, &self.pool, LOAD_PREKEY, id as i32, self.device_id)?;
        let row = row
            .map(|stored| self.open_at("prekeys", "key", &id.to_be_bytes(), &stored))
            .transpose()?;
        Ok(row.map(Bytes::from))
    }

    async fn remove_prekey(&self, id: u32) -> Result<()> {
        execute_sql!(&self.pool, DELETE_PREKEY, id as i32, self.device_id)?;
        Ok(())
    }

    async fn get_max_prekey_id(&self) -> Result<u32> {
        let max = scalar_one_sql!(i32, &self.pool, MAX_PREKEY_ID, self.device_id)?;
        Ok(max as u32)
    }

    // --- Signed PreKeys ---

    async fn store_signed_prekey(&self, id: u32, record: &[u8]) -> Result<()> {
        let sealed = self.seal_at("signed_prekeys", "record", &id.to_be_bytes(), record)?;
        execute_sql!(
            &self.pool,
            STORE_SIGNED_PREKEY,
            id as i32,
            &sealed[..],
            self.device_id
        )?;
        Ok(())
    }

    async fn load_signed_prekey(&self, id: u32) -> Result<Option<Vec<u8>>> {
        let row = scalar_optional_sql!(
            Vec<u8>,
            &self.pool,
            LOAD_SIGNED_PREKEY,
            id as i32,
            self.device_id
        )?;
        row.map(|stored| self.open_at("signed_prekeys", "record", &id.to_be_bytes(), &stored))
            .transpose()
    }

    async fn load_all_signed_prekeys(&self) -> Result<Vec<(u32, Vec<u8>)>> {
        let rows = row_all_sql!(
            (i32, Vec<u8>),
            &self.pool,
            LOAD_ALL_SIGNED_PREKEYS,
            self.device_id
        )?;
        rows.into_iter()
            .map(|(id, stored)| {
                let record =
                    self.open_at("signed_prekeys", "record", &id.to_be_bytes(), &stored)?;
                Ok((id as u32, record))
            })
            .collect()
    }

    async fn remove_signed_prekey(&self, id: u32) -> Result<()> {
        execute_sql!(&self.pool, REMOVE_SIGNED_PREKEY, id as i32, self.device_id)?;
        Ok(())
    }

    // --- Sender Keys ---

    async fn put_sender_key(&self, address: &str, record: &[u8]) -> Result<()> {
        let sealed = self.seal_at("sender_keys", "record", address.as_bytes(), record)?;
        execute_sql!(
            &self.pool,
            PUT_SENDER_KEY,
            address,
            &sealed[..],
            self.device_id
        )?;
        Ok(())
    }

    async fn get_sender_key(&self, address: &str) -> Result<Option<Vec<u8>>> {
        let row =
            scalar_optional_sql!(Vec<u8>, &self.pool, GET_SENDER_KEY, address, self.device_id)?;
        row.map(|stored| self.open_at("sender_keys", "record", address.as_bytes(), &stored))
            .transpose()
    }

    async fn delete_sender_key(&self, address: &str) -> Result<()> {
        execute_sql!(&self.pool, DELETE_SENDER_KEY, address, self.device_id)?;
        Ok(())
    }

    // --- Batches (#104): one transaction or one query per chunk, see `signal_sql` ---

    async fn put_identities_batch(&self, identities: &[(Arc<str>, [u8; 32])]) -> Result<()> {
        signal_sql::put_identities(&self.pool, &self.cipher, self.device_id, identities).await
    }

    async fn delete_identities_batch(&self, addresses: &[Arc<str>]) -> Result<()> {
        signal_sql::delete_identities(&self.pool, self.device_id, addresses).await
    }

    async fn put_sessions_batch(&self, sessions: &[(Arc<str>, Bytes)]) -> Result<()> {
        signal_sql::put_sessions(&self.pool, &self.cipher, self.device_id, sessions).await
    }

    async fn get_sessions_batch(&self, addresses: &[Arc<str>]) -> Result<Vec<(Arc<str>, Bytes)>> {
        signal_sql::get_sessions(&self.pool, &self.cipher, self.device_id, addresses).await
    }

    async fn delete_sessions_batch(&self, addresses: &[Arc<str>]) -> Result<()> {
        signal_sql::delete_sessions(&self.pool, self.device_id, addresses).await
    }

    async fn store_prekeys_batch(&self, keys: &[(u32, Bytes)], uploaded: bool) -> Result<()> {
        signal_sql::store_prekeys(&self.pool, &self.cipher, self.device_id, keys, uploaded).await
    }

    async fn load_prekeys_batch(&self, ids: &[u32]) -> Result<Vec<(u32, Bytes)>> {
        signal_sql::load_prekeys(&self.pool, &self.cipher, self.device_id, ids).await
    }

    async fn remove_prekeys_batch(&self, ids: &[u32]) -> Result<()> {
        signal_sql::remove_prekeys(&self.pool, self.device_id, ids).await
    }

    async fn put_sender_keys_batch(&self, sender_keys: &[(Arc<str>, Bytes)]) -> Result<()> {
        signal_sql::put_sender_keys(&self.pool, &self.cipher, self.device_id, sender_keys).await
    }

    async fn delete_sender_keys_batch(&self, addresses: &[Arc<str>]) -> Result<()> {
        signal_sql::delete_sender_keys(&self.pool, self.device_id, addresses).await
    }
}
