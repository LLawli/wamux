//! One-shot conversion of the bincode blobs from the released whatsapp-rust
//! 0.7.0 (crates.io) to git main `f4d73ebe` (#30).
//!
//! bincode-standard is POSITIONAL (see `blob_migration`). Two of the blobs wamux
//! persists changed layout between the release and main:
//!
//! | blob | column | what changed |
//! |---|---|---|
//! | `Device` | `device.data` | `server_client_expiration` inserted MID-struct (before `read_receipts_disabled`); `server_cert_chain` gained `signature_verified` |
//! | `HashState` | `app_state_versions.state_data` | `bootstrapped` appended |
//! | `AppStateSyncKey` | `app_state_keys.key_data` | unchanged, verified byte-compatible |
//!
//! The new fields start at the value upstream documents for a record written
//! before they existed, never a guess:
//!
//! - `signature_verified: false`: an unmarked legacy chain may not authorize
//!   Noise IK, so the next connect does one XX and caches a verified chain.
//! - `server_client_expiration: None`: the server pushes it when a build is being
//!   retired, and the common case is that it never has.
//! - `bootstrapped: false`: the collection bootstraps once more rather than ask
//!   for patches its ltHash may not accept.
//!
//! Compiled only under `migrate-0-7-main`, which links the released `wacore`
//! 0.7.0 as `wacore070` next to the git one.

use super::blob_migration::{BlobMigration, BlobMigrationSteps, MigrateError};

/// The table `blob_migration_runner` drives.
pub const MIGRATE_0_7_MAIN: BlobMigrationSteps = BlobMigrationSteps {
    name: "whatsapp-rust 0.7.0 -> main f4d73ebe",
    device: migrate_device_blob,
    hash_state: migrate_hash_state_blob,
    sync_key: verify_sync_key_blob,
};

/// `device.data`.
pub fn migrate_device_blob(bytes: &[u8]) -> Result<BlobMigration, MigrateError> {
    let _ = bytes;
    todo!("#30: 0.7.0 Device -> main Device, field by field")
}

/// `app_state_versions.state_data`.
pub fn migrate_hash_state_blob(bytes: &[u8]) -> Result<BlobMigration, MigrateError> {
    let _ = bytes;
    todo!("#30: 0.7.0 HashState -> main HashState")
}

/// `app_state_keys.key_data` needs no rewrite; checked anyway, because
/// "unchanged" is a claim about the data.
pub fn verify_sync_key_blob(bytes: &[u8]) -> Result<(), MigrateError> {
    let _ = bytes;
    todo!("#30: decode as main AppStateSyncKey, whole")
}

#[cfg(test)]
mod tests;
