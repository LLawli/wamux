//! `HashState` and `AppStateSyncKey` <-> their wire messages. Field numbers
//! match whatsapp-rust's sqlite backend, so these blobs are interchangeable
//! with its rows.

use prost::Message as _;
use wacore::appstate::hash::HashState;
use wacore::store::error::StoreError;
use wacore::store::traits::AppStateSyncKey;

use super::wire::{AppStateSyncKeyWire, HashStateWire};
use super::{decode_error, fixed_bytes, wrong_length};

/// The app-state master key is the HKDF input for every collection's sub-keys.
const APP_STATE_KEY_LEN: usize = 32;

/// `app_state_versions.state_data`.
pub fn encode_hash_state(state: &HashState) -> Vec<u8> {
    HashStateWire {
        version: state.version,
        hash: state.hash.to_vec(),
        // Collected into the generated BTreeMap: sorted, so the encode is
        // deterministic regardless of the HashMap's iteration order.
        index_value_map: state
            .index_value_map
            .iter()
            .map(|(index, value)| (index.clone(), value.clone()))
            .collect(),
        mac_mismatch_fatal: state.mac_mismatch_fatal,
        bootstrapped: state.bootstrapped,
    }
    .encode_to_vec()
}

pub fn decode_hash_state(bytes: &[u8]) -> Result<HashState, StoreError> {
    let wire = HashStateWire::decode(bytes).map_err(decode_error)?;
    Ok(HashState {
        version: wire.version,
        hash: fixed_bytes("hash_state.hash", wire.hash)?,
        index_value_map: wire.index_value_map.into_iter().collect(),
        mac_mismatch_fatal: wire.mac_mismatch_fatal,
        bootstrapped: wire.bootstrapped,
    })
}

/// `app_state_keys.key_data`.
pub fn encode_app_state_sync_key(key: &AppStateSyncKey) -> Vec<u8> {
    AppStateSyncKeyWire {
        key_data: key.key_data.clone(),
        fingerprint: key.fingerprint.clone(),
        timestamp: key.timestamp,
    }
    .encode_to_vec()
}

/// Rejects a master key that is not 32 bytes, like the upstream backend: sub-keys
/// derived from one would fail every MAC later, far from the cause.
pub fn decode_app_state_sync_key(bytes: &[u8]) -> Result<AppStateSyncKey, StoreError> {
    let wire = AppStateSyncKeyWire::decode(bytes).map_err(decode_error)?;
    if wire.key_data.len() != APP_STATE_KEY_LEN {
        return Err(wrong_length(
            "app_state_sync_key.key_data",
            APP_STATE_KEY_LEN,
            wire.key_data.len(),
        ));
    }
    Ok(AppStateSyncKey {
        key_data: wire.key_data,
        fingerprint: wire.fingerprint,
        timestamp: wire.timestamp,
    })
}
