//! `AppSyncStore` statements shared by the one-row methods and the batched
//! ones (#104). Kept out of `app_sync_store.rs` for the `*_store.rs` rule.

use std::collections::HashMap;
use std::sync::LazyLock;

use wacore::appstate::hash::HashState;
use wacore::appstate::processor::AppStateMutationMAC;
use wacore::store::error::Result;

use super::batch_sql::{READ_CHUNK, in_placeholders, padded_chunks};
use super::{SqlPool, SqlTx};
use crate::storage::blob_codec::encode_hash_state;

pub(super) const SET_VERSION: &str =
    "INSERT INTO app_state_versions (name, state_data, device_id) VALUES ($1, $2, $3)
     ON CONFLICT (name, device_id) DO UPDATE SET state_data = EXCLUDED.state_data";
pub(super) const PUT_MUTATION_MAC: &str =
    "INSERT INTO app_state_mutation_macs (name, version, index_mac, value_mac, device_id)
     VALUES ($1, $2, $3, $4, $5)
     ON CONFLICT (name, index_mac, device_id)
     DO UPDATE SET version = EXCLUDED.version, value_mac = EXCLUDED.value_mac";
pub(super) const DELETE_MUTATION_MAC: &str = "DELETE FROM app_state_mutation_macs
     WHERE name = $1 AND index_mac = $2 AND device_id = $3";

// Same text on every call and never varies by deployment: built once.
static SELECT_MUTATION_MACS: LazyLock<String> = LazyLock::new(|| {
    let list = in_placeholders(3, READ_CHUNK);
    format!(
        "SELECT index_mac, value_mac FROM app_state_mutation_macs
         WHERE name = $1 AND device_id = $2 AND index_mac IN ({list})"
    )
});

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
