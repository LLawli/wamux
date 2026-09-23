//! What every one-shot bincode blob migration shares: the result and error
//! types, the whole-slice decode, and the table of steps a migration plugs into
//! the runner (`blob_migration_runner`).
//!
//! bincode-standard is POSITIONAL: it stores no field names, so
//! `#[serde(default)]` does NOT rescue a struct that gained a field, and every
//! upstream field addition to a persisted type needs one of these. #31 tracks
//! moving the blobs to a field-tagged format so that stops being true.
//!
//! Compiled only under a migration feature: each one links a second `wacore`
//! (`wacore06`, `wacore070`) to read the old layout.

use thiserror::Error;

/// What one blob needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobMigration {
    /// Already decodes as the target layout: a re-run, or a store written after
    /// the upgrade. Makes the whole migration idempotent.
    AlreadyCurrent,
    /// Decoded as the old layout and re-encoded as the new one. These are the
    /// bytes to write back.
    Rewritten(Vec<u8>),
}

#[derive(Debug, Error)]
pub enum MigrateError {
    /// Neither layout can read the blob. Never migrate past this: the row is
    /// something other than what the column claims, and overwriting it would
    /// destroy whatever it actually is.
    #[error("{label}: unreadable by both layouts (old said: {as_old}; new said: {as_new})")]
    Unreadable {
        label: &'static str,
        as_old: String,
        as_new: String,
    },
    /// Decoded, but the bytes we produced do not read back. A bug here, not bad
    /// data; the caller must abort rather than write.
    #[error("{label}: re-encoded but the result does not decode back: {cause}")]
    RoundTrip { label: &'static str, cause: String },
    #[error(
        "{label}: the old protobuf bytes did not survive the round trip ({before} -> {after} bytes)"
    )]
    AccountBytes {
        label: &'static str,
        before: usize,
        after: usize,
    },
    #[error("{label}: could not rebuild the {field} key material: {cause}")]
    KeyMaterial {
        label: &'static str,
        field: &'static str,
        cause: String,
    },
}

/// One migration, as the runner sees it: a name for the report and one function
/// per blob column. A table of plain `fn`s rather than a trait, so each
/// migration module exports a `const` and a bin is one line.
pub struct BlobMigrationSteps {
    /// Shown in `--help` and the report, e.g. `"0.7.0 -> main f4d73ebe"`.
    pub name: &'static str,
    /// `device.data`.
    pub device: fn(&[u8]) -> Result<BlobMigration, MigrateError>,
    /// `app_state_versions.state_data`.
    pub hash_state: fn(&[u8]) -> Result<BlobMigration, MigrateError>,
    /// `app_state_keys.key_data`: verified, never rewritten.
    pub sync_key: fn(&[u8]) -> Result<(), MigrateError>,
}

fn config() -> bincode::config::Configuration {
    bincode::config::standard()
}

/// Decode consuming EVERY byte. A partial decode means the blob is not really
/// this type, it just happens to start like one, and accepting it would let an
/// old `HashState` masquerade as a current one (they differ by trailing bytes).
pub fn decode_whole<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    let (value, used) =
        bincode::serde::decode_from_slice::<T, _>(bytes, config()).map_err(|e| e.to_string())?;
    if used != bytes.len() {
        return Err(format!(
            "trailing bytes: consumed {used} of {}",
            bytes.len()
        ));
    }
    Ok(value)
}

/// Encode `value` and prove the bytes decode back as `T` before anyone writes
/// them. Every migration ends here, so no rewrite can ship unreadable bytes.
pub fn encode_checked<T>(value: &T, label: &'static str) -> Result<Vec<u8>, MigrateError>
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let out =
        bincode::serde::encode_to_vec(value, config()).map_err(|e| MigrateError::RoundTrip {
            label,
            cause: e.to_string(),
        })?;
    decode_whole::<T>(&out).map_err(|cause| MigrateError::RoundTrip { label, cause })?;
    Ok(out)
}

/// Carry an opaque value (`Jid`, a key pair half) across a version boundary
/// through its own bincode. The round trip PROVES the two layouts agree instead
/// of assuming it.
pub fn reserde<A, B>(value: &A, label: &'static str, field: &'static str) -> Result<B, MigrateError>
where
    A: serde::Serialize,
    B: serde::de::DeserializeOwned,
{
    let bytes =
        bincode::serde::encode_to_vec(value, config()).map_err(|e| MigrateError::KeyMaterial {
            label,
            field,
            cause: e.to_string(),
        })?;
    decode_whole::<B>(&bytes).map_err(|cause| MigrateError::KeyMaterial {
        label,
        field,
        cause,
    })
}

#[cfg(test)]
mod blob_migration_shared_tests {
    use super::*;

    #[test]
    fn decode_whole_rejects_a_blob_with_bytes_left_over() {
        // Two u8s: a u8 decode would stop after one and report success.
        let blob = bincode::serde::encode_to_vec((7u8, 9u8), config()).unwrap();
        assert!(decode_whole::<u8>(&blob).unwrap_err().contains("trailing"));
        assert_eq!(decode_whole::<(u8, u8)>(&blob).unwrap(), (7, 9));
    }

    #[test]
    fn encode_checked_returns_bytes_that_decode_back() {
        let out = encode_checked(&(42u64, true), "probe").unwrap();
        assert_eq!(decode_whole::<(u64, bool)>(&out).unwrap(), (42, true));
    }

    #[test]
    fn reserde_fails_loudly_when_the_layouts_disagree() {
        let err = reserde::<(u8, u8), u8>(&(1, 2), "probe", "field").unwrap_err();
        assert!(matches!(
            err,
            MigrateError::KeyMaterial { field: "field", .. }
        ));
    }
}
