//! `SignalStore` statements, shared by the one-row methods and the batches
//! (#104). All key material is raw bytes.

use std::sync::LazyLock;

use crate::storage::batch_chunks::{READ_CHUNK, in_placeholders};

pub const PUT_IDENTITY: &str = "INSERT INTO identities (address, key, device_id)
     VALUES ($1, $2, $3)
     ON CONFLICT (address, device_id) DO UPDATE SET key = EXCLUDED.key";
pub const LOAD_IDENTITY: &str = "SELECT key FROM identities WHERE address = $1 AND device_id = $2";
pub const DELETE_IDENTITY: &str = "DELETE FROM identities WHERE address = $1 AND device_id = $2";

pub const PUT_SESSION: &str = "INSERT INTO sessions (address, record, device_id)
     VALUES ($1, $2, $3)
     ON CONFLICT (address, device_id) DO UPDATE SET record = EXCLUDED.record";
pub const GET_SESSION: &str = "SELECT record FROM sessions WHERE address = $1 AND device_id = $2";
pub const DELETE_SESSION: &str = "DELETE FROM sessions WHERE address = $1 AND device_id = $2";

pub const STORE_PREKEY: &str = "INSERT INTO prekeys (id, key, uploaded, device_id)
     VALUES ($1, $2, $3, $4)
     ON CONFLICT (id, device_id) DO UPDATE
     SET key = EXCLUDED.key, uploaded = EXCLUDED.uploaded";
pub const LOAD_PREKEY: &str = "SELECT key FROM prekeys WHERE id = $1 AND device_id = $2";
pub const DELETE_PREKEY: &str = "DELETE FROM prekeys WHERE id = $1 AND device_id = $2";
pub const MAX_PREKEY_ID: &str = "SELECT COALESCE(MAX(id), 0) FROM prekeys WHERE device_id = $1";

/// One id at a time, the shape SQLite and Turso use (no array bind). UPDATE,
/// never upsert: see `SignalStore::mark_prekeys_uploaded`. Postgres binds an
/// array instead and keeps that statement in `sql::prekeys_sql`.
pub const MARK_PREKEY_UPLOADED: &str =
    "UPDATE prekeys SET uploaded = TRUE WHERE id = $1 AND device_id = $2";

pub const STORE_SIGNED_PREKEY: &str =
    "INSERT INTO signed_prekeys (id, record, device_id) VALUES ($1, $2, $3)
             ON CONFLICT (id, device_id) DO UPDATE SET record = EXCLUDED.record";
pub const LOAD_SIGNED_PREKEY: &str =
    "SELECT record FROM signed_prekeys WHERE id = $1 AND device_id = $2";
pub const LOAD_ALL_SIGNED_PREKEYS: &str =
    "SELECT id, record FROM signed_prekeys WHERE device_id = $1";
pub const REMOVE_SIGNED_PREKEY: &str =
    "DELETE FROM signed_prekeys WHERE id = $1 AND device_id = $2";

pub const PUT_SENDER_KEY: &str = "INSERT INTO sender_keys (address, record, device_id)
     VALUES ($1, $2, $3)
     ON CONFLICT (address, device_id) DO UPDATE SET record = EXCLUDED.record";
pub const GET_SENDER_KEY: &str =
    "SELECT record FROM sender_keys WHERE address = $1 AND device_id = $2";
pub const DELETE_SENDER_KEY: &str = "DELETE FROM sender_keys WHERE address = $1 AND device_id = $2";

// The statement text is the same for every call and never varies by
// deployment, so it is built once, not per call.
pub static SELECT_SESSIONS: LazyLock<String> = LazyLock::new(|| {
    let list = in_placeholders(2, READ_CHUNK);
    format!("SELECT address, record FROM sessions WHERE device_id = $1 AND address IN ({list})")
});
pub static SELECT_PREKEYS: LazyLock<String> = LazyLock::new(|| {
    let list = in_placeholders(2, READ_CHUNK);
    format!("SELECT id, key FROM prekeys WHERE device_id = $1 AND id IN ({list})")
});
