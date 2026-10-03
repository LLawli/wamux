//! `ProtocolStore` statements, shared by the one-row methods and the batched
//! ones (#104): sender-key tracking, LID-PN mappings, base keys, the device
//! registry, tc tokens (the atomic writers are in `tc_token`) and the sent cache.

use std::sync::LazyLock;

use crate::storage::batch_chunks::{READ_CHUNK, in_placeholders};

pub const GET_SENDER_KEY_DEVICES: &str = "SELECT device_jid, has_key FROM sender_key_devices
             WHERE group_jid = $1 AND device_id = $2";
pub const SET_SENDER_KEY_STATUS: &str =
    "INSERT INTO sender_key_devices (group_jid, device_jid, has_key, device_id, updated_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (group_jid, device_jid, device_id)
                 DO UPDATE SET has_key = EXCLUDED.has_key, updated_at = EXCLUDED.updated_at";
pub const CLEAR_SENDER_KEY_DEVICES: &str =
    "DELETE FROM sender_key_devices WHERE group_jid = $1 AND device_id = $2";
pub const DELETE_SENDER_KEY_DEVICE_ROW: &str =
    "DELETE FROM sender_key_devices WHERE device_jid = $1 AND device_id = $2";
pub const CLEAR_ALL_SENDER_KEY_DEVICES: &str =
    "DELETE FROM sender_key_devices WHERE device_id = $1";

pub const GET_LID_MAPPING: &str =
    "SELECT lid, phone_number, created_at, learning_source, updated_at
             FROM lid_pn_mapping WHERE lid = $1 AND device_id = $2";
pub const GET_PN_MAPPING: &str = "SELECT lid, phone_number, created_at, learning_source, updated_at
             FROM lid_pn_mapping WHERE phone_number = $1 AND device_id = $2
             ORDER BY updated_at DESC LIMIT 1";
pub const GET_ALL_LID_MAPPINGS: &str =
    "SELECT lid, phone_number, created_at, learning_source, updated_at
             FROM lid_pn_mapping WHERE device_id = $1";
pub const PUT_LID_MAPPING: &str = "INSERT INTO lid_pn_mapping
        (lid, phone_number, created_at, learning_source, updated_at, device_id)
     VALUES ($1, $2, $3, $4, $5, $6)
     ON CONFLICT (lid, device_id) DO UPDATE SET
        phone_number = EXCLUDED.phone_number,
        learning_source = EXCLUDED.learning_source,
        updated_at = EXCLUDED.updated_at";

pub const SAVE_BASE_KEY: &str = "INSERT INTO base_keys (address, message_id, base_key, device_id, created_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (address, message_id, device_id) DO UPDATE SET base_key = EXCLUDED.base_key";
pub const GET_BASE_KEY: &str = "SELECT base_key FROM base_keys
             WHERE address = $1 AND message_id = $2 AND device_id = $3";
pub const DELETE_BASE_KEY: &str =
    "DELETE FROM base_keys WHERE address = $1 AND message_id = $2 AND device_id = $3";
/// The keepalive sweep calls this every cycle; the trait default was Ok(0),
/// so base_keys grew without bound.
pub const DELETE_EXPIRED_BASE_KEYS: &str =
    "DELETE FROM base_keys WHERE created_at < $1 AND device_id = $2";

pub const UPDATE_DEVICE_LIST: &str = "INSERT INTO device_registry
        (user_id, devices_json, timestamp, phash, device_id, updated_at, raw_id)
     VALUES ($1, $2, $3, $4, $5, $6, $7)
     ON CONFLICT (user_id, device_id) DO UPDATE SET
        devices_json = EXCLUDED.devices_json,
        timestamp = EXCLUDED.timestamp,
        phash = EXCLUDED.phash,
        updated_at = EXCLUDED.updated_at,
        raw_id = EXCLUDED.raw_id";
pub const GET_DEVICES: &str = "SELECT user_id, devices_json, timestamp, phash, raw_id
             FROM device_registry WHERE user_id = $1 AND device_id = $2";
pub const DELETE_DEVICES: &str =
    "DELETE FROM device_registry WHERE user_id = $1 AND device_id = $2";

pub const GET_TC_TOKEN: &str = "SELECT token, token_timestamp, sender_timestamp
             FROM tc_tokens WHERE jid = $1 AND device_id = $2";
pub const PUT_TC_TOKEN: &str = "INSERT INTO tc_tokens
                (jid, token, token_timestamp, sender_timestamp, device_id, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (jid, device_id) DO UPDATE SET
                token = EXCLUDED.token,
                token_timestamp = EXCLUDED.token_timestamp,
                sender_timestamp = EXCLUDED.sender_timestamp,
                updated_at = EXCLUDED.updated_at";
pub const DELETE_TC_TOKEN: &str = "DELETE FROM tc_tokens WHERE jid = $1 AND device_id = $2";
pub const GET_ALL_TC_TOKEN_JIDS: &str = "SELECT jid FROM tc_tokens WHERE device_id = $1";
/// Two independent windows, both of which must be stale before the row goes:
/// the received token (expired or byte-empty) AND the sender bucket (expired
/// or never set). Pruning on the token alone would drop recent sender state
/// that the retry path still needs. Mirrors the reference `SqliteStore`.
/// `length()` counts bytes for a Postgres `bytea` too, so it stands in for the
/// Postgres-only byte-length function.
pub const DELETE_EXPIRED_TC_TOKENS: &str = "DELETE FROM tc_tokens
             WHERE (length(token) = 0 OR token_timestamp < $1)
               AND (sender_timestamp IS NULL OR sender_timestamp < $2)
               AND device_id = $3";

/// REPLACE semantics in the reference reset created_at; mirror that.
pub const STORE_SENT_MESSAGE: &str =
    "INSERT INTO sent_messages (chat_jid, message_id, payload, device_id, created_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (chat_jid, message_id, device_id)
             DO UPDATE SET payload = EXCLUDED.payload, created_at = EXCLUDED.created_at";
pub const GET_SENT_MESSAGE: &str = "SELECT payload FROM sent_messages
             WHERE chat_jid = $1 AND message_id = $2 AND device_id = $3";
pub const DELETE_SENT_MESSAGE: &str = "DELETE FROM sent_messages
                 WHERE chat_jid = $1 AND message_id = $2 AND device_id = $3";
pub const DELETE_EXPIRED_SENT_MESSAGES: &str =
    "DELETE FROM sent_messages WHERE created_at < $1 AND device_id = $2";

// Same text on every call and never varies by deployment: built once.
pub static SELECT_DEVICES: LazyLock<String> = LazyLock::new(|| {
    let list = in_placeholders(2, READ_CHUNK);
    format!(
        "SELECT user_id, devices_json, timestamp, phash, raw_id
         FROM device_registry WHERE device_id = $1 AND user_id IN ({list})"
    )
});
pub static SELECT_TC_TOKENS: LazyLock<String> = LazyLock::new(|| {
    let list = in_placeholders(2, READ_CHUNK);
    format!(
        "SELECT jid, token, token_timestamp, sender_timestamp
         FROM tc_tokens WHERE device_id = $1 AND jid IN ({list})"
    )
});
