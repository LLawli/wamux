//! `AppSyncStore` for the turso family, method for method the sql family's
//! (#106). Sync keys and version state are protobuf blobs (`blob_codec`, #31);
//! MACs are raw bytes.

use std::collections::HashMap;

use async_trait::async_trait;
use wacore::appstate::hash::HashState;
use wacore::appstate::processor::AppStateMutationMAC;
use wacore::store::error::Result;
use wacore::store::traits::{AppStateSyncKey, AppSyncStore};

use super::exec::binds;
use super::row_values::first_blob;
use super::{TursoBackend, TursoTx, app_sync_sql};
use crate::storage::blob_cipher::joined_row;
use crate::storage::blob_codec::{
    decode_app_state_sync_key, decode_hash_state, encode_app_state_sync_key, encode_hash_state,
};
use crate::storage::statements::app_sync::{
    CLEAR_MUTATION_MACS, DELETE_VERSION, GET_MUTATION_MAC, GET_SYNC_KEY, GET_VERSION,
    LATEST_SYNC_KEY_ID, SET_SYNC_KEY, SET_VERSION,
};

#[async_trait]
impl AppSyncStore for TursoBackend {
    async fn get_sync_key(&self, key_id: &[u8]) -> Result<Option<AppStateSyncKey>> {
        let row = self
            .conn
            .fetch_optional(GET_SYNC_KEY, binds![key_id, self.device_id])
            .await?;
        match first_blob(row)? {
            None => Ok(None),
            Some(stored) => {
                let bytes = self.open_at("app_state_keys", "key_data", key_id, &stored)?;
                Ok(Some(decode_app_state_sync_key(&bytes)?))
            }
        }
    }

    async fn set_sync_key(&self, key_id: &[u8], key: AppStateSyncKey) -> Result<()> {
        let data = encode_app_state_sync_key(&key);
        let data = self.seal_at("app_state_keys", "key_data", key_id, &data)?;
        let binds = binds![key_id, data, self.device_id];
        self.conn.execute(SET_SYNC_KEY, binds).await?;
        Ok(())
    }

    // `main` made absence meaningful: no row means "never synced" (the lib
    // bootstraps from a snapshot), while a row at version 0 means a collection
    // that synced and is legitimately empty (the lib asks for patches).
    // Collapsing both into `HashState::default()` made an empty collection
    // re-request a snapshot forever, so a missing row now reads as `None`.
    async fn get_version(&self, name: &str) -> Result<Option<HashState>> {
        let row = self
            .conn
            .fetch_optional(GET_VERSION, binds![name, self.device_id])
            .await?;
        match first_blob(row)? {
            None => Ok(None),
            Some(stored) => {
                let name = name.as_bytes();
                let bytes = self.open_at("app_state_versions", "state_data", name, &stored)?;
                Ok(Some(decode_hash_state(&bytes)?))
            }
        }
    }

    /// Forget a collection's version, returning it to the never-synced state.
    /// A missing row is a no-op, not an error; only this device's row is
    /// touched.
    async fn delete_version(&self, name: &str) -> Result<()> {
        let binds = binds![name, self.device_id];
        self.conn.execute(DELETE_VERSION, binds).await?;
        Ok(())
    }

    async fn set_version(&self, name: &str, state: HashState) -> Result<()> {
        let data = encode_hash_state(&state);
        let data = self.seal_at("app_state_versions", "state_data", name.as_bytes(), &data)?;
        let binds = binds![name, data, self.device_id];
        self.conn.execute(SET_VERSION, binds).await?;
        Ok(())
    }

    async fn put_mutation_macs(
        &self,
        name: &str,
        version: u64,
        mutations: &[AppStateMutationMAC],
    ) -> Result<()> {
        let tx = TursoTx::begin(&self.conn).await?;
        app_sync_sql::put_macs_in(&tx, &self.cipher, self.device_id, name, version, mutations)
            .await?;
        tx.commit().await
    }

    async fn get_mutation_mac(&self, name: &str, index_mac: &[u8]) -> Result<Option<Vec<u8>>> {
        let binds = binds![name, index_mac, self.device_id];
        let row = self.conn.fetch_optional(GET_MUTATION_MAC, binds).await?;
        let row_key = joined_row(&[name.as_bytes(), index_mac]);
        first_blob(row)?
            .map(|stored| self.open_at("app_state_mutation_macs", "value_mac", &row_key, &stored))
            .transpose()
    }

    async fn delete_mutation_macs(&self, name: &str, index_macs: &[Vec<u8>]) -> Result<()> {
        let tx = TursoTx::begin(&self.conn).await?;
        app_sync_sql::delete_macs_in(&tx, self.device_id, name, index_macs).await?;
        tx.commit().await
    }

    /// Wipe a collection's MAC store. The lib calls this on snapshot re-sync:
    /// the snapshot rebuilds the ltHash from scratch, so a MAC left over from
    /// the pre-snapshot timeline would corrupt the next patch's ltHash.
    async fn clear_mutation_macs(&self, name: &str) -> Result<()> {
        let binds = binds![name, self.device_id];
        self.conn.execute(CLEAR_MUTATION_MACS, binds).await?;
        Ok(())
    }

    async fn get_latest_sync_key_id(&self) -> Result<Option<Vec<u8>>> {
        let binds = binds![self.device_id];
        first_blob(self.conn.fetch_optional(LATEST_SYNC_KEY_ID, binds).await?)
    }

    // --- Throughput overrides (#104), see `app_sync_sql` ---

    async fn get_mutation_macs(
        &self,
        name: &str,
        index_macs: &[[u8; 32]],
    ) -> Result<HashMap<[u8; 32], Vec<u8>>> {
        app_sync_sql::get_mutation_macs(&self.conn, &self.cipher, self.device_id, name, index_macs)
            .await
    }

    async fn commit_patch(
        &self,
        name: &str,
        state: HashState,
        removed_index_macs: &[Vec<u8>],
        added: &[AppStateMutationMAC],
    ) -> Result<()> {
        let device_id = self.device_id;
        let (conn, cipher) = (&self.conn, &self.cipher);
        let removed = removed_index_macs;
        app_sync_sql::commit_patch(conn, cipher, device_id, name, state, removed, added).await
    }
}
