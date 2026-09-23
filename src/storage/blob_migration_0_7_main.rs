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

use super::blob_migration::{
    BlobMigration, BlobMigrationSteps, MigrateError, decode_whole, reserde,
};

/// The table `blob_migration_runner` drives.
pub const MIGRATE_0_7_MAIN: BlobMigrationSteps = BlobMigrationSteps {
    name: "whatsapp-rust 0.7.0 -> main f4d73ebe",
    device: migrate_device_blob,
    hash_state: migrate_hash_state_blob,
    sync_key: verify_sync_key_blob,
};

/// `device.data`. See the module docs for why a plain re-encode is not enough.
pub fn migrate_device_blob(bytes: &[u8]) -> Result<BlobMigration, MigrateError> {
    const LABEL: &str = "device.data";

    let as_new = match decode_whole::<wacore::store::Device>(bytes) {
        Ok(_) => return Ok(BlobMigration::AlreadyCurrent),
        Err(e) => e,
    };
    let old: wacore070::store::Device =
        decode_whole(bytes).map_err(|as_old| MigrateError::Unreadable {
            label: LABEL,
            as_old,
            as_new: as_new.clone(),
        })?;

    let new = bridge_device(&old, LABEL)?;
    let out = bincode::serde::encode_to_vec(&new, bincode::config::standard()).map_err(|e| {
        MigrateError::RoundTrip {
            label: LABEL,
            cause: e.to_string(),
        }
    })?;
    decode_whole::<wacore::store::Device>(&out).map_err(|cause| MigrateError::RoundTrip {
        label: LABEL,
        cause,
    })?;
    Ok(BlobMigration::Rewritten(out))
}

/// `app_state_versions.state_data`. One appended bool, but bincode is positional
/// so the old blob still ends one byte short of what main reads.
pub fn migrate_hash_state_blob(bytes: &[u8]) -> Result<BlobMigration, MigrateError> {
    const LABEL: &str = "app_state_versions.state_data";
    use wacore::appstate::hash::HashState as New;
    use wacore070::appstate::hash::HashState as Old;

    let as_new = match decode_whole::<New>(bytes) {
        Ok(_) => return Ok(BlobMigration::AlreadyCurrent),
        Err(e) => e,
    };
    let old: Old = decode_whole(bytes).map_err(|as_old| MigrateError::Unreadable {
        label: LABEL,
        as_old,
        as_new: as_new.clone(),
    })?;

    let new = New {
        version: old.version,
        hash: old.hash,
        index_value_map: old.index_value_map,
        mac_mismatch_fatal: old.mac_mismatch_fatal,
        // A run that persisted this state under 0.7.0 never went through the
        // paging bootstrap `bootstrapped` tracks (the field didn't exist), so
        // the honest starting value is "not bootstrapped": the collection
        // bootstraps once more rather than trust a ltHash the field never
        // vouched for.
        bootstrapped: false,
    };
    let out = bincode::serde::encode_to_vec(&new, bincode::config::standard()).map_err(|e| {
        MigrateError::RoundTrip {
            label: LABEL,
            cause: e.to_string(),
        }
    })?;
    decode_whole::<New>(&out).map_err(|cause| MigrateError::RoundTrip {
        label: LABEL,
        cause,
    })?;
    Ok(BlobMigration::Rewritten(out))
}

/// `app_state_keys.key_data` needs no rewrite; checked anyway, because
/// "unchanged" is a claim about the data.
pub fn verify_sync_key_blob(bytes: &[u8]) -> Result<(), MigrateError> {
    const LABEL: &str = "app_state_keys.key_data";
    decode_whole::<wacore::store::traits::AppStateSyncKey>(bytes).map_err(|as_new| {
        MigrateError::Unreadable {
            label: LABEL,
            as_old: "not attempted".to_string(),
            as_new,
        }
    })?;
    Ok(())
}

/// Field-by-field, never `..Default::default()`: `Device::default()` GENERATES
/// FRESH KEYS, so a field forgotten here would silently replace an account's
/// identity instead of failing to compile.
fn bridge_device(
    old: &wacore070::store::Device,
    label: &'static str,
) -> Result<wacore::store::Device, MigrateError> {
    Ok(wacore::store::Device {
        pn: reserde(&old.pn, label, "pn")?,
        lid: reserde(&old.lid, label, "lid")?,
        registration_id: old.registration_id,
        noise_key: bridge_key_pair(&old.noise_key, label, "noise_key")?,
        identity_key: bridge_key_pair(&old.identity_key, label, "identity_key")?,
        signed_pre_key: bridge_key_pair(&old.signed_pre_key, label, "signed_pre_key")?,
        signed_pre_key_id: old.signed_pre_key_id,
        signed_pre_key_signature: old.signed_pre_key_signature,
        adv_secret_key: old.adv_secret_key,
        account: bridge_account(old, label)?,
        push_name: old.push_name.clone(),
        app_version_primary: old.app_version_primary,
        app_version_secondary: old.app_version_secondary,
        app_version_tertiary: old.app_version_tertiary,
        app_version_last_fetched_ms: old.app_version_last_fetched_ms,
        // #[serde(skip)] on both sides: absent from the blob, rebuilt at load.
        device_props: Default::default(),
        client_profile: Default::default(),
        nct_salt_sync_seen: false,
        edge_routing_info: old.edge_routing_info.clone(),
        props_hash: old.props_hash.clone(),
        next_pre_key_id: old.next_pre_key_id,
        // 0.7.0 already had this field: carry it, never reset it. Resetting
        // the prekey watermark on a real 0.7.0 store would re-offer keys the
        // client already uploaded.
        first_unupload_pre_key_id: old.first_unupload_pre_key_id,
        server_has_prekeys: old.server_has_prekeys,
        nct_salt: old.nct_salt.clone(),
        server_cert_chain: old.server_cert_chain.as_ref().map(|c| {
            wacore::store::CachedServerCertChain {
                intermediate: bridge_cert(&c.intermediate),
                leaf: bridge_cert(&c.leaf),
                // Upstream's rule for a legacy record: an unmarked chain may
                // not authorize Noise IK, so the next connect does one XX and
                // caches a chain this field can call verified.
                signature_verified: false,
            }
        }),
        // 0.7.0 already had these three: carry them, never reset them. A
        // real 0.7.0 store may have logged in, migrated to LID, or rotated
        // its signed prekey since pairing, and resetting any of that here is
        // data loss.
        login_counter: old.login_counter,
        lid_migrated: old.lid_migrated,
        last_signed_pre_key_rotation_ms: old.last_signed_pre_key_rotation_ms,
        // The field main inserted. `None` until the server says otherwise,
        // which is the common case: the stanza is sent when a build is being
        // retired, not on every connect.
        server_client_expiration: None,
        read_receipts_disabled: old.read_receipts_disabled,
    })
}

/// Rebuild a Signal keypair from its raw 32-byte halves. Deliberately explicit
/// rather than a serde hop: these bytes ARE the account's identity, so the code
/// that moves them should be readable at a glance.
fn bridge_key_pair(
    old: &wacore070::libsignal::protocol::KeyPair,
    label: &'static str,
    field: &'static str,
) -> Result<wacore::libsignal::protocol::KeyPair, MigrateError> {
    use wacore::libsignal::protocol::{KeyPair, PrivateKey, PublicKey};
    let private = old.private_key.serialize();
    let public =
        PublicKey::from_djb_public_key_bytes(old.public_key.public_key_bytes()).map_err(|e| {
            MigrateError::KeyMaterial {
                label,
                field,
                cause: format!("public half: {e}"),
            }
        })?;
    let private = PrivateKey::deserialize(&private[..]).map_err(|e| MigrateError::KeyMaterial {
        label,
        field,
        cause: format!("private half: {e}"),
    })?;
    Ok(KeyPair::new(public, private))
}

fn bridge_cert(old: &wacore070::store::CachedNoiseCert) -> wacore::store::CachedNoiseCert {
    wacore::store::CachedNoiseCert {
        key: old.key,
        not_before: old.not_before,
        not_after: old.not_after,
    }
}

/// The `ADVSignedDeviceIdentity` Rust type is buffa-generated on both sides of
/// this boundary (0.7.0 already made the prost -> buffa move), but it comes from
/// two different `waproto` crate versions, so the bridge still goes through the
/// stable on-disk form: the protobuf bytes (`account_serde` on either side).
fn bridge_account(
    old: &wacore070::store::Device,
    label: &'static str,
) -> Result<
    Option<std::sync::Arc<whatsapp_rust::waproto::whatsapp::ADVSignedDeviceIdentity>>,
    MigrateError,
> {
    let Some(account) = old.account.as_ref() else {
        return Ok(None);
    };
    let bytes = wacore070::store::device::account_serde::to_bytes(account);
    let decoded = wacore::store::device::account_serde::from_bytes(&bytes).map_err(|e| {
        MigrateError::KeyMaterial {
            label,
            field: "account",
            cause: format!("buffa rejected the 0.7.0 bytes: {e}"),
        }
    })?;
    // The pairing identity is what proves this device to WhatsApp. Assert the
    // bytes are unchanged rather than trusting that two generators agree.
    let reencoded = wacore::store::device::account_serde::to_bytes(&decoded);
    if reencoded != bytes {
        return Err(MigrateError::AccountBytes {
            label,
            before: bytes.len(),
            after: reencoded.len(),
        });
    }
    Ok(Some(std::sync::Arc::new(decoded)))
}

#[cfg(test)]
mod tests;
