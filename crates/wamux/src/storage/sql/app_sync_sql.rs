//! `AppSyncStore` statements shared by the one-row methods and the batched
//! ones (#104). Kept out of `app_sync_store.rs` for the `*_store.rs` rule.

use std::collections::HashMap;

use wacore::appstate::hash::HashState;
use wacore::appstate::processor::AppStateMutationMAC;
use wacore::store::error::Result;

use super::{SqlPool, SqlTx};
use crate::storage::batch_chunks::padded_chunks;
use crate::storage::blob_codec::encode_hash_state;
use crate::storage::statements::app_sync::{
    DELETE_MUTATION_MAC, PUT_MUTATION_MAC, SELECT_MUTATION_MACS, SET_VERSION,
};

// `mut tx: &mut SqlTx`: `execute_sql!(in tx, ..)` takes `&mut tx`, so the binding itself is mut.
pub(super) async fn put_macs_in(
    mut tx: &mut SqlTx<'_>,
    device_id: i32,
    name: &str,
    version: u64,
    added: &[AppStateMutationMAC],
) -> Result<()> {
    for m in added {
        let (index, value) = (m.index_mac.as_slice(), m.value_mac.as_slice());
        execute_sql!(in tx, PUT_MUTATION_MAC, name, version as i64, index, value, device_id)?;
    }
    Ok(())
}

pub(super) async fn delete_macs_in(
    mut tx: &mut SqlTx<'_>,
    device_id: i32,
    name: &str,
    index_macs: &[Vec<u8>],
) -> Result<()> {
    for index_mac in index_macs {
        execute_sql!(in tx, DELETE_MUTATION_MAC, name, index_mac.as_slice(), device_id)?;
    }
    Ok(())
}

/// The new version, the removed MACs and the added MACs in ONE transaction.
/// Three separate writes let a crash between them leave the new version next to
/// the old MACs, and the next patch's ltHash was then computed from the wrong
/// set. Removals run before additions, as the trait default did.
pub(super) async fn commit_patch(
    pool: &SqlPool,
    device_id: i32,
    name: &str,
    state: HashState,
    removed_index_macs: &[Vec<u8>],
    added: &[AppStateMutationMAC],
) -> Result<()> {
    let (version, data) = (state.version, encode_hash_state(&state));
    let mut tx = SqlTx::begin(pool).await?;
    execute_sql!(in tx, SET_VERSION, name, &data, device_id)?;
    delete_macs_in(&mut tx, device_id, name, removed_index_macs).await?;
    put_macs_in(&mut tx, device_id, name, version, added).await?;
    tx.commit().await
}

/// Only the asked collection and account; absent MACs are left out.
pub(super) async fn get_mutation_macs(
    pool: &SqlPool,
    device_id: i32,
    name: &str,
    index_macs: &[[u8; 32]],
) -> Result<HashMap<[u8; 32], Vec<u8>>> {
    let mut found: HashMap<[u8; 32], Vec<u8>> = HashMap::with_capacity(index_macs.len());
    for chunk in padded_chunks(index_macs) {
        let rows = rows_in_list_sql!(
            (Vec<u8>, Vec<u8>),
            pool,
            SELECT_MUTATION_MACS.as_str(),
            [name, device_id],
            chunk.iter().map(|index_mac| index_mac.as_slice())
        )?;
        // A stored index that is not 32 bytes cannot be one that was asked for.
        found.extend(rows.into_iter().filter_map(|(index, mac)| {
            let key: [u8; 32] = index.try_into().ok()?;
            Some((key, mac))
        }));
    }
    Ok(found)
}
