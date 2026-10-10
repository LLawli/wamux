//! `AppSyncStore` helpers shared by the one-row methods and the batched ones
//! (#106, the turso twin of `sql::app_sync_sql`). Kept out of
//! `app_sync_store.rs` for the `*_store.rs` rule.

use std::collections::HashMap;

use wacore::appstate::hash::HashState;
use wacore::appstate::processor::AppStateMutationMAC;
use wacore::store::error::Result;

use super::exec::{binds, with_list};
use super::row_values::blob;
use super::{TursoConn, TursoTx};
use crate::storage::batch_chunks::padded_chunks;
use crate::storage::blob_cipher::{BlobCipher, joined_row};
use crate::storage::blob_codec::encode_hash_state;
use crate::storage::statements::app_sync::{
    DELETE_MUTATION_MAC, PUT_MUTATION_MAC, SELECT_MUTATION_MACS, SET_VERSION,
};

/// The AAD row key of the MACs is `name | 0 | index_mac`: the index stays in the
/// clear (it is the lookup key) but is bound to its MAC.
const MAC_TABLE: &str = "app_state_mutation_macs";
const MAC_COLUMN: &str = "value_mac";

pub(super) async fn put_macs_in(
    tx: &TursoTx<'_>,
    cipher: &BlobCipher,
    device_id: i32,
    name: &str,
    version: u64,
    added: &[AppStateMutationMAC],
) -> Result<()> {
    for m in added {
        let index = m.index_mac.as_slice();
        let row = joined_row(&[name.as_bytes(), index]);
        let value = cipher.seal_at(device_id, MAC_TABLE, MAC_COLUMN, &row, &m.value_mac)?;
        let binds = binds![name, version as i64, index, value, device_id];
        tx.execute(PUT_MUTATION_MAC, binds).await?;
    }
    Ok(())
}

pub(super) async fn delete_macs_in(
    tx: &TursoTx<'_>,
    device_id: i32,
    name: &str,
    index_macs: &[Vec<u8>],
) -> Result<()> {
    for index_mac in index_macs {
        let binds = binds![name, index_mac.as_slice(), device_id];
        tx.execute(DELETE_MUTATION_MAC, binds).await?;
    }
    Ok(())
}

/// The new version, the removed MACs and the added MACs in ONE transaction.
/// Three separate writes let a crash between them leave the new version next to
/// the old MACs, and the next patch's ltHash was then computed from the wrong
/// set. Removals run before additions, as the trait default did.
pub(super) async fn commit_patch(
    conn: &TursoConn,
    cipher: &BlobCipher,
    device_id: i32,
    name: &str,
    state: HashState,
    removed_index_macs: &[Vec<u8>],
    added: &[AppStateMutationMAC],
) -> Result<()> {
    let version = state.version;
    let data = encode_hash_state(&state);
    let data = cipher.seal_at(
        device_id,
        "app_state_versions",
        "state_data",
        name.as_bytes(),
        &data,
    )?;
    let tx = TursoTx::begin(conn).await?;
    tx.execute(SET_VERSION, binds![name, data, device_id])
        .await?;
    delete_macs_in(&tx, device_id, name, removed_index_macs).await?;
    put_macs_in(&tx, cipher, device_id, name, version, added).await?;
    tx.commit().await
}

/// Only the asked collection and account; absent MACs are left out.
pub(super) async fn get_mutation_macs(
    conn: &TursoConn,
    cipher: &BlobCipher,
    device_id: i32,
    name: &str,
    index_macs: &[[u8; 32]],
) -> Result<HashMap<[u8; 32], Vec<u8>>> {
    let mut found: HashMap<[u8; 32], Vec<u8>> = HashMap::with_capacity(index_macs.len());
    for chunk in padded_chunks(index_macs) {
        let list = chunk.iter().map(|index_mac| index_mac.as_slice().into());
        let binds = with_list(binds![name, device_id], list);
        for row in conn.fetch_all(SELECT_MUTATION_MACS.as_str(), binds).await? {
            // A stored index that is not 32 bytes cannot be one that was asked for.
            let index = blob(&row, 0)?;
            let Ok(key) = <[u8; 32]>::try_from(index.as_slice()) else {
                continue;
            };
            let row_key = joined_row(&[name.as_bytes(), &index]);
            let stored = blob(&row, 1)?;
            found.insert(
                key,
                cipher.open_at(device_id, MAC_TABLE, MAC_COLUMN, &row_key, &stored)?,
            );
        }
    }
    Ok(found)
}
