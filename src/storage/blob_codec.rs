//! Byte formats shared by every storage engine.
//!
//! These are NOT engine details: the blobs must be byte-identical across
//! Postgres and SQLite, so a store dumped from one engine can be loaded by the
//! other. Keeping the codec in one place is what makes that claim testable.
//!
//! The three structured columns are protobuf (`proto/store/blobs.proto`, #31),
//! converted here at the storage boundary so the wacore types stay untouched.
//! They used to be bincode-standard, which is positional: every field upstream
//! appended made every stored blob unreadable until a one-shot migration
//! linked the old wacore next to the new one. Protobuf is field-tagged, so a
//! field a blob predates now decodes as its default.
//!
//! The decode builds each wacore struct with a struct literal and never with
//! `..Default::default()`. That is deliberate twice over: `Device::default()`
//! mints fresh keys, and a literal turns an upstream field addition into a
//! compile error here instead of a silently dropped value.

mod app_state_wire;
mod device_wire;
/// Also the fixtures `bincode_upgrade`'s tests build legacy blobs from.
#[cfg(test)]
pub(crate) mod tests;
mod wire;

pub use app_state_wire::{
    decode_app_state_sync_key, decode_hash_state, encode_app_state_sync_key, encode_hash_state,
};
pub use device_wire::{decode_device, encode_device};

use wacore::store::error::StoreError;

/// A length check failed: a fixed-size field (a key, a signature, the ltHash)
/// came back the wrong size. Names the field and both sizes, never the bytes.
fn wrong_length(field: &str, expected: usize, got: usize) -> StoreError {
    StoreError::Serialization(format!("{field}: expected {expected} bytes, got {got}").into())
}

/// Copy a stored byte field into the fixed-size array wacore holds it in.
fn fixed_bytes<const N: usize>(field: &str, bytes: Vec<u8>) -> Result<[u8; N], StoreError> {
    let got = bytes.len();
    bytes.try_into().map_err(|_| wrong_length(field, N, got))
}

fn decode_error(e: prost::DecodeError) -> StoreError {
    StoreError::Serialization(Box::new(e))
}

/// Unix seconds, matching the reference's `wacore::time::now_secs()` semantics.
pub fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
