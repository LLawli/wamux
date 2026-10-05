//! `AppSyncStore` for the SQL family. Sync keys and version state are protobuf
//! blobs (`blob_codec`, #31); MACs are raw bytes.

use std::collections::HashMap;

use async_trait::async_trait;
use wacore::appstate::hash::HashState;
use wacore::appstate::processor::AppStateMutationMAC;
use wacore::store::error::Result;
use wacore::store::traits::{AppStateSyncKey, AppSyncStore};

use super::app_sync_sql;
use super::{SqlBackend, SqlTx};
use crate::storage::blob_codec::{
    decode_app_state_sync_key, decode_hash_state, encode_app_state_sync_key, encode_hash_state,
};
use crate::storage::statements::app_sync::{
    CLEAR_MUTATION_MACS, DELETE_VERSION, GET_MUTATION_MAC, GET_SYNC_KEY, GET_VERSION,
    LATEST_SYNC_KEY_ID, SET_SYNC_KEY, SET_VERSION,
};

#[async_trait]
impl AppSyncStore for SqlBackend {
    async fn get_sync_key(&self, key_id: &[u8]) -> Result<Option<AppStateSyncKey>> {
        let row = scalar_optional_sql!(Vec<u8>, &self.pool, GET_SYNC_KEY, key_id, self.device_id)?;
        match row {
            None => Ok(None),
            Some(bytes) => Ok(Some(decode_app_state_sync_key(&bytes)?)),
        }
    }

    async fn set_sync_key(&self, key_id: &[u8], key: AppStateSyncKey) -> Result<()> {
        let data = encode_app_state_sync_key(&key);
        execute_sql!(&self.pool, SET_SYNC_KEY, key_id, &data, self.device_id)?;
        Ok(())
    }

    // `main` made absence meaningful: no row means "never synced" (the lib
    // bootstraps from a snapshot), while a row at version 0 means a collection
    // that synced and is legitimately empty (the lib asks for patches).
    // Collapsing both into `HashState::default()` made an empty collection
    // re-request a snapshot forever, so a missing row now reads as `None`.
    async fn get_version(&self, name: &str) -> Result<Option<HashState>> {
        let row = scalar_optional_sql!(Vec<u8>, &self.pool, GET_VERSION, name, self.device_id)?;
        match row {
            None => Ok(None),
            Some(bytes) => Ok(Some(decode_hash_state(&bytes)?)),
        }
    }

    /// Forget a collection's version, returning it to the never-synced state.
    /// A missing row is a no-op, not an error; only this device's row is
    /// touched.
    async fn delete_version(&self, name: &str) -> Result<()> {
        execute_sql!(&self.pool, DELETE_VERSION, name, self.device_id)?;
        Ok(())
    }

    async fn set_version(&self, name: &str, state: HashState) -> Result<()> {
        let data = encode_hash_state(&state);
        execute_sql!(&self.pool, SET_VERSION, name, &data, self.device_id)?;
        Ok(())
    }

    async fn put_mutation_macs(
        &self,
        name: &str,
        version: u64,
        mutations: &[AppStateMutationMAC],
    ) -> Result<()> {
        let mut tx = SqlTx::begin(&self.pool).await?;
        app_sync_sql::put_macs_in(&mut tx, self.device_id, name, version, mutations).await?;
        tx.commit().await
    }

    async fn get_mutation_mac(&self, name: &str, index_mac: &[u8]) -> Result<Option<Vec<u8>>> {
        scalar_optional_sql!(
            Vec<u8>,
            &self.pool,
            GET_MUTATION_MAC,
            name,
            index_mac,
            self.device_id
        )
    }

    async fn delete_mutation_macs(&self, name: &str, index_macs: &[Vec<u8>]) -> Result<()> {
        let mut tx = SqlTx::begin(&self.pool).await?;
        app_sync_sql::delete_macs_in(&mut tx, self.device_id, name, index_macs).await?;
        tx.commit().await
    }

    /// Wipe a collection's MAC store. The lib calls this on snapshot re-sync:
    /// the snapshot rebuilds the ltHash from scratch, so a MAC left over from
    /// the pre-snapshot timeline would corrupt the next patch's ltHash.
    async fn clear_mutation_macs(&self, name: &str) -> Result<()> {
        execute_sql!(&self.pool, CLEAR_MUTATION_MACS, name, self.device_id)?;
        Ok(())
    }

    async fn get_latest_sync_key_id(&self) -> Result<Option<Vec<u8>>> {
        scalar_optional_sql!(Vec<u8>, &self.pool, LATEST_SYNC_KEY_ID, self.device_id)
    }

    // --- Throughput overrides (#104), see `app_sync_sql` ---

    async fn get_mutation_macs(
        &self,
        name: &str,
        index_macs: &[[u8; 32]],
    ) -> Result<HashMap<[u8; 32], Vec<u8>>> {
        app_sync_sql::get_mutation_macs(&self.pool, self.device_id, name, index_macs).await
    }

    async fn commit_patch(
        &self,
        name: &str,
        state: HashState,
        removed_index_macs: &[Vec<u8>],
        added: &[AppStateMutationMAC],
    ) -> Result<()> {
        let device_id = self.device_id;
        app_sync_sql::commit_patch(
            &self.pool,
            device_id,
            name,
            state,
            removed_index_macs,
            added,
        )
        .await
    }
}
