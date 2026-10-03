//! `AppSyncStore` statements, shared by the one-row methods and the batched
//! ones (#104). Sync keys and version state are protobuf blobs; MACs are raw bytes.

use std::sync::LazyLock;

use crate::storage::batch_chunks::{READ_CHUNK, in_placeholders};

pub const GET_SYNC_KEY: &str =
    "SELECT key_data FROM app_state_keys WHERE key_id = $1 AND device_id = $2";
pub const SET_SYNC_KEY: &str =
    "INSERT INTO app_state_keys (key_id, key_data, device_id) VALUES ($1, $2, $3)
             ON CONFLICT (key_id, device_id) DO UPDATE SET key_data = EXCLUDED.key_data";
pub const LATEST_SYNC_KEY_ID: &str =
    "SELECT key_id FROM app_state_keys WHERE device_id = $1 ORDER BY key_id DESC LIMIT 1";

pub const GET_VERSION: &str =
    "SELECT state_data FROM app_state_versions WHERE name = $1 AND device_id = $2";
pub const DELETE_VERSION: &str =
    "DELETE FROM app_state_versions WHERE name = $1 AND device_id = $2";
pub const SET_VERSION: &str =
    "INSERT INTO app_state_versions (name, state_data, device_id) VALUES ($1, $2, $3)
     ON CONFLICT (name, device_id) DO UPDATE SET state_data = EXCLUDED.state_data";

pub const PUT_MUTATION_MAC: &str =
    "INSERT INTO app_state_mutation_macs (name, version, index_mac, value_mac, device_id)
     VALUES ($1, $2, $3, $4, $5)
     ON CONFLICT (name, index_mac, device_id)
     DO UPDATE SET version = EXCLUDED.version, value_mac = EXCLUDED.value_mac";
pub const GET_MUTATION_MAC: &str = "SELECT value_mac FROM app_state_mutation_macs
             WHERE name = $1 AND index_mac = $2 AND device_id = $3";
pub const DELETE_MUTATION_MAC: &str = "DELETE FROM app_state_mutation_macs
     WHERE name = $1 AND index_mac = $2 AND device_id = $3";
pub const CLEAR_MUTATION_MACS: &str =
    "DELETE FROM app_state_mutation_macs WHERE name = $1 AND device_id = $2";

// Same text on every call and never varies by deployment: built once.
pub static SELECT_MUTATION_MACS: LazyLock<String> = LazyLock::new(|| {
    let list = in_placeholders(3, READ_CHUNK);
    format!(
        "SELECT index_mac, value_mac FROM app_state_mutation_macs
         WHERE name = $1 AND device_id = $2 AND index_mac IN ({list})"
    )
});
