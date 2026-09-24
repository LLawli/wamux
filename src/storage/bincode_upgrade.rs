//! The last blob migration (#31): bincode-standard -> protobuf, run by the
//! daemon itself when it opens the store, before any account loads.
//!
//! Why this one is automatic when the two before it were one-shot bins: those
//! had to read an OLD layout, which meant linking the old `wacore` next to the
//! new one. This reads the CURRENT types as bincode and writes them as protobuf,
//! so the running build is all it needs.
//!
//! `blob_format` (one row, created by the SQL migration as `bincode`) says which
//! format the three columns hold. It is what makes this safe to run on every
//! start: nothing is ever guessed from the bytes, a converted store is skipped
//! at the cost of one SELECT, and the rows and the marker change in one
//! transaction, so a store is never half one format and half the other.
//!
//! The contract, kept from the one-shot runner it replaces:
//! - **Plan first.** Every blob converts and is proved before anything is
//!   written; one that will not aborts the run with nothing changed.
//! - **Proved, not assumed.** Each new blob is decoded back and compared with
//!   the value the old one held. For a `Device` that comparison covers every
//!   persisted field, key material included.
//! - **Counts only in logs and errors.** These blobs are Signal key material.
//!
//! Delete this module, the `bincode` dependency and the check in the engines'
//! `run_migrations` once every deployed store reports `protobuf`.

#[cfg(test)]
mod tests;

use thiserror::Error;
use wacore::appstate::hash::HashState;
use wacore::store::Device;
use wacore::store::traits::AppStateSyncKey;

use super::blob_codec::{
    decode_app_state_sync_key, decode_device, decode_hash_state, encode_app_state_sync_key,
    encode_device, encode_hash_state,
};
use super::bootstrapped_repair::repair_inherited_bootstrap;

/// `blob_format.format` before and after the conversion.
pub const BLOB_FORMAT_BINCODE: &str = "bincode";
pub const BLOB_FORMAT_PROTOBUF: &str = "protobuf";

#[derive(Debug, Error)]
pub enum BincodeUpgradeError {
    /// The row is not the bincode it should be. Never convert past this: it is
    /// something other than what the column claims, and overwriting it would
    /// destroy whatever it actually is.
    #[error("{row}: not readable as bincode ({cause}); nothing was converted")]
    Unreadable { row: String, cause: String },
    /// Converted, but the protobuf bytes do not read back as the same value. A
    /// bug in the codec, not bad data.
    #[error(
        "{row}: the protobuf blob does not read back as the same value ({cause}); nothing was converted"
    )]
    RoundTrip { row: String, cause: String },
    /// A marker this build does not know, e.g. written by a newer build.
    #[error("blob_format is '{0}', expected '{BLOB_FORMAT_BINCODE}' or '{BLOB_FORMAT_PROTOBUF}'")]
    UnknownFormat(String),
    #[error("{context}: {source}")]
    Database {
        context: &'static str,
        #[source]
        source: sqlx::Error,
    },
}

/// Every rewrite, decided before a byte is written.
#[derive(Debug, Default)]
pub struct BincodeUpgradePlan {
    /// `(device_id, new device.data)`.
    pub devices: Vec<(i32, Vec<u8>)>,
    /// `(device_id, name, new state_data)`.
    pub versions: Vec<(i32, String, Vec<u8>)>,
    /// `(device_id, key_id, new key_data)`.
    pub sync_keys: Vec<(i32, Vec<u8>, Vec<u8>)>,
    /// `(device_id, name)` of every collection `repair_inherited_bootstrap`
    /// marked. Names only: they are WhatsApp's fixed collection names.
    pub repaired_bootstrap: Vec<(i32, String)>,
}

/// The rows as read, one tuple per row, in the shape of each table's key.
pub struct LegacyBlobRows {
    pub devices: Vec<(i32, Vec<u8>)>,
    pub versions: Vec<(i32, String, Vec<u8>)>,
    pub sync_keys: Vec<(i32, Vec<u8>, Vec<u8>)>,
}

/// Plan the whole store, or fail naming the first row that would not convert.
pub fn plan_bincode_upgrade(
    rows: LegacyBlobRows,
) -> Result<BincodeUpgradePlan, BincodeUpgradeError> {
    let mut plan = BincodeUpgradePlan::default();
    for (device_id, blob) in rows.devices {
        let row = format!("device.data of device {device_id}");
        plan.devices
            .push((device_id, upgrade_device_blob(&blob, row)?));
    }
    for (device_id, name, blob) in rows.versions {
        let row = format!("app_state_versions '{name}' of device {device_id}");
        let (new, repaired) = upgrade_hash_state_blob(&blob, row)?;
        if repaired {
            plan.repaired_bootstrap.push((device_id, name.clone()));
        }
        plan.versions.push((device_id, name, new));
    }
    for (device_id, key_id, blob) in rows.sync_keys {
        let row = format!("an app_state_keys row of device {device_id}");
        plan.sync_keys
            .push((device_id, key_id, upgrade_sync_key_blob(&blob, row)?));
    }
    Ok(plan)
}

/// One `device.data`. Proved by comparing every persisted field: bincode itself
/// serializes exactly those, so the old value's bincode and the round-tripped
/// value's bincode are equal only if nothing was lost or changed.
pub fn upgrade_device_blob(blob: &[u8], row: String) -> Result<Vec<u8>, BincodeUpgradeError> {
    let old: Device = decode_whole(blob, &row)?;
    let new = encode_device(&old);
    let back = decode_device(&new).map_err(|e| round_trip(&row, e.to_string()))?;
    if encode_bincode(&back, &row)? != encode_bincode(&old, &row)? {
        return Err(round_trip(&row, "a persisted field changed".into()));
    }
    Ok(new)
}

/// One `app_state_versions.state_data`, plus the bootstrapped repair (see
/// `bootstrapped_repair`); the bool says whether the repair marked it.
///
/// Compared field by field (`HashState` holds a HashMap, so its bincode depends
/// on iteration order), against the old value with only the repair applied:
/// any other difference is a codec bug.
pub fn upgrade_hash_state_blob(
    blob: &[u8],
    row: String,
) -> Result<(Vec<u8>, bool), BincodeUpgradeError> {
    let mut expected: HashState = decode_whole(blob, &row)?;
    let repaired = repair_inherited_bootstrap(&mut expected);
    let new = encode_hash_state(&expected);
    let back = decode_hash_state(&new).map_err(|e| round_trip(&row, e.to_string()))?;
    let same = back.version == expected.version
        && back.hash == expected.hash
        && back.index_value_map == expected.index_value_map
        && back.mac_mismatch_fatal == expected.mac_mismatch_fatal
        && back.bootstrapped == expected.bootstrapped;
    if !same {
        return Err(round_trip(&row, "a field changed".into()));
    }
    Ok((new, repaired))
}

/// One `app_state_keys.key_data`. A key that is not 32 bytes fails here, since
/// the protobuf decode rejects it: better at startup than as a MAC failure.
pub fn upgrade_sync_key_blob(blob: &[u8], row: String) -> Result<Vec<u8>, BincodeUpgradeError> {
    let old: AppStateSyncKey = decode_whole(blob, &row)?;
    let new = encode_app_state_sync_key(&old);
    let back = decode_app_state_sync_key(&new).map_err(|e| round_trip(&row, e.to_string()))?;
    let same = back.key_data == old.key_data
        && back.fingerprint == old.fingerprint
        && back.timestamp == old.timestamp;
    if !same {
        return Err(round_trip(&row, "a field changed".into()));
    }
    Ok(new)
}

/// Decode consuming EVERY byte. A partial decode means the blob only starts
/// like this type; accepting it would convert something that is not one.
fn decode_whole<T: serde::de::DeserializeOwned>(
    blob: &[u8],
    row: &str,
) -> Result<T, BincodeUpgradeError> {
    let unreadable = |cause: String| BincodeUpgradeError::Unreadable {
        row: row.to_string(),
        cause,
    };
    let (value, used) =
        bincode::serde::decode_from_slice::<T, _>(blob, bincode::config::standard())
            .map_err(|e| unreadable(e.to_string()))?;
    if used != blob.len() {
        return Err(unreadable(format!(
            "trailing bytes: consumed {used} of {}",
            blob.len()
        )));
    }
    Ok(value)
}

fn encode_bincode<T: serde::Serialize>(
    value: &T,
    row: &str,
) -> Result<Vec<u8>, BincodeUpgradeError> {
    bincode::serde::encode_to_vec(value, bincode::config::standard())
        .map_err(|e| round_trip(row, e.to_string()))
}

fn round_trip(row: &str, cause: String) -> BincodeUpgradeError {
    BincodeUpgradeError::RoundTrip {
        row: row.to_string(),
        cause,
    }
}

/// What the marker says to do. `Ok(false)`: already protobuf, nothing to do.
pub fn needs_bincode_upgrade(format: &str) -> Result<bool, BincodeUpgradeError> {
    match format {
        BLOB_FORMAT_BINCODE => Ok(true),
        BLOB_FORMAT_PROTOBUF => Ok(false),
        other => Err(BincodeUpgradeError::UnknownFormat(other.to_string())),
    }
}

/// Wraps a raw `sqlx::Error` with what was being attempted.
pub fn upgrade_db_error(context: &'static str) -> impl FnOnce(sqlx::Error) -> BincodeUpgradeError {
    move |source| BincodeUpgradeError::Database { context, source }
}

/// One line per conversion, counts and collection names only. The repaired list
/// is what the deploy is checked against (see `bootstrapped_repair`).
pub fn log_upgrade(plan: &BincodeUpgradePlan) {
    tracing::info!(
        devices = plan.devices.len(),
        app_state_versions = plan.versions.len(),
        app_state_keys = plan.sync_keys.len(),
        repaired_bootstrap = plan.repaired_bootstrap.len(),
        "store blobs converted from bincode to protobuf (#31)"
    );
    for (device_id, name) in &plan.repaired_bootstrap {
        tracing::info!(device_id, collection = %name, "app-state collection marked bootstrapped");
    }
}
