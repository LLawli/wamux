//! The SQL half of `storage::bincode_upgrade`. Postgres reads the marker with
//! `FOR UPDATE` (kept in `sql::bincode_upgrade`); SQLite and Turso have no row
//! lock and need none: one connection, one daemon per file.

pub const SELECT_BLOB_FORMAT: &str = "SELECT format FROM blob_format WHERE id = 1";

pub const MARK_BLOB_FORMAT: &str = "UPDATE blob_format SET format = $1 WHERE id = 1";

pub const SELECT_LEGACY_DEVICES: &str = "SELECT device_id, data FROM device ORDER BY device_id";

pub const SELECT_LEGACY_VERSIONS: &str =
    "SELECT device_id, name, state_data FROM app_state_versions ORDER BY device_id, name";

pub const SELECT_LEGACY_SYNC_KEYS: &str =
    "SELECT device_id, key_id, key_data FROM app_state_keys ORDER BY device_id, key_id";

pub const UPDATE_DEVICE_BLOB: &str = "UPDATE device SET data = $1 WHERE device_id = $2";

pub const UPDATE_VERSION_BLOB: &str =
    "UPDATE app_state_versions SET state_data = $1 WHERE device_id = $2 AND name = $3";

pub const UPDATE_SYNC_KEY_BLOB: &str =
    "UPDATE app_state_keys SET key_data = $1 WHERE device_id = $2 AND key_id = $3";
