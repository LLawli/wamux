//! One-shot conversion of the bincode blobs from whatsapp-rust 0.6 to 0.7.
//!
//! bincode-standard is POSITIONAL: it stores no field names, so `#[serde(default)]`
//! does NOT rescue a struct that gained a field. Two of the blobs wamux persists
//! gained one between 0.6.0 and 0.7.0 and no longer decode:
//!
//! | blob | column | what changed |
//! |---|---|---|
//! | `Device` | `device.data` | `first_unupload_pre_key_id` inserted MID-struct, plus four appended fields |
//! | `HashState` | `app_state_versions.state_data` | `mac_mismatch_fatal` appended |
//! | `AppStateSyncKey` | `app_state_keys.key_data` | unchanged, verified byte-compatible |
//!
//! Both failures are loud (a decode error, never silent garbage), which is why a
//! migration is possible at all. Without it every paired account is lost.
//!
//! This module is compiled only under the `migrate-0-7` feature, which is what
//! pulls in the second `wacore` (0.6.0 alongside 0.7.0). The two are
//! semver-incompatible, so cargo links both, and that is the whole trick.
//!
//! Delete this module, its feature, and the `wacore06` dependency once every
//! deployment has run the migration.

use thiserror::Error;

/// What one blob needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobMigration {
    /// Already decodes as 0.7: a re-run, or a store written after the upgrade.
    /// Makes the whole migration idempotent.
    AlreadyCurrent,
    /// Decoded as 0.6 and re-encoded as 0.7. These are the bytes to write back.
    Rewritten(Vec<u8>),
}

#[derive(Debug, Error)]
pub enum MigrateError {
    /// Neither version can read the blob. Never migrate past this: the row is
    /// something other than what the column claims, and overwriting it would
    /// destroy whatever it actually is.
    #[error("{label}: unreadable by both 0.6 and 0.7 (0.6 said: {as_0_6}; 0.7 said: {as_0_7})")]
    Unreadable {
        label: &'static str,
        as_0_6: String,
        as_0_7: String,
    },
    /// Decoded, but the 0.7 bytes we produced do not read back. A bug here, not
    /// bad data; the caller must abort rather than write.
    #[error("{label}: re-encoded to 0.7 but the result does not decode back: {cause}")]
    RoundTrip { label: &'static str, cause: String },
    #[error(
        "{label}: 0.6 protobuf bytes did not survive the buffa round trip ({before} -> {after} bytes)"
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

fn config() -> bincode::config::Configuration {
    bincode::config::standard()
}

/// Decode consuming EVERY byte. A partial decode means the blob is not really
/// this type — it just happens to start like one — and accepting it would let a
/// 0.6 `HashState` masquerade as an already-current 0.7 one (the two differ by a
/// single trailing byte).
fn decode_whole<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
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

/// `device.data`. See the module docs for why a plain re-encode is not enough.
pub fn migrate_device_blob(bytes: &[u8]) -> Result<BlobMigration, MigrateError> {
    const LABEL: &str = "device.data";

    let as_0_7 = match decode_whole::<wacore::store::Device>(bytes) {
        Ok(_) => return Ok(BlobMigration::AlreadyCurrent),
        Err(e) => e,
    };
    let old: wacore06::store::Device =
        decode_whole(bytes).map_err(|as_0_6| MigrateError::Unreadable {
            label: LABEL,
            as_0_6,
            as_0_7: as_0_7.clone(),
        })?;

    let new = bridge_device(&old, LABEL)?;
    let out =
        bincode::serde::encode_to_vec(&new, config()).map_err(|e| MigrateError::RoundTrip {
            label: LABEL,
            cause: e.to_string(),
        })?;
    decode_whole::<wacore::store::Device>(&out).map_err(|cause| MigrateError::RoundTrip {
        label: LABEL,
        cause,
    })?;
    Ok(BlobMigration::Rewritten(out))
}

/// `app_state_versions.state_data`. One appended bool, but bincode is positional
/// so the old blob still ends one byte short of what 0.7 reads.
pub fn migrate_hash_state_blob(bytes: &[u8]) -> Result<BlobMigration, MigrateError> {
    const LABEL: &str = "app_state_versions.state_data";
    use wacore::appstate::hash::HashState as New;
    use wacore06::appstate::hash::HashState as Old;

    let as_0_7 = match decode_whole::<New>(bytes) {
        Ok(_) => return Ok(BlobMigration::AlreadyCurrent),
        Err(e) => e,
    };
    let old: Old = decode_whole(bytes).map_err(|as_0_6| MigrateError::Unreadable {
        label: LABEL,
        as_0_6,
        as_0_7: as_0_7.clone(),
    })?;

    let new = New {
        version: old.version,
        hash: old.hash,
        index_value_map: old.index_value_map,
        // The flag means "this collection's ltHash is beyond repair". A store
        // migrated from 0.6 has never had a patch fail that check, because the
        // check did not exist, so the honest starting value is false.
        mac_mismatch_fatal: false,
    };
    let out =
        bincode::serde::encode_to_vec(&new, config()).map_err(|e| MigrateError::RoundTrip {
            label: LABEL,
            cause: e.to_string(),
        })?;
    decode_whole::<New>(&out).map_err(|cause| MigrateError::RoundTrip {
        label: LABEL,
        cause,
    })?;
    Ok(BlobMigration::Rewritten(out))
}

/// `app_state_keys.key_data` needs no rewrite: the struct is unchanged and the
/// bytes decode identically under both versions. Checked anyway, because
/// "unchanged" is a claim about the data, and the migration is the last cheap
/// place to find out it is false.
pub fn verify_sync_key_blob(bytes: &[u8]) -> Result<(), MigrateError> {
    const LABEL: &str = "app_state_keys.key_data";
    decode_whole::<wacore::store::traits::AppStateSyncKey>(bytes).map_err(|as_0_7| {
        MigrateError::Unreadable {
            label: LABEL,
            as_0_6: "not attempted".to_string(),
            as_0_7,
        }
    })?;
    Ok(())
}

/// Field-by-field, never `..Default::default()`: `Device::default()` GENERATES
/// FRESH KEYS, so a field forgotten here would silently replace an account's
/// identity instead of failing to compile.
fn bridge_device(
    old: &wacore06::store::Device,
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
        server_has_prekeys: old.server_has_prekeys,
        nct_salt: old.nct_salt.clone(),
        server_cert_chain: old.server_cert_chain.as_ref().map(|c| {
            wacore::store::CachedServerCertChain {
                intermediate: bridge_cert(&c.intermediate),
                leaf: bridge_cert(&c.leaf),
            }
        }),
        // --- the five fields 0.7 added, each at the value the lib documents as
        // --- "this device predates the field", never a guess.
        // 0 = legacy device; the first prekey upload initialises the watermark.
        first_unupload_pre_key_id: 0,
        // The counter is an anti-abuse signal the server tolerates restarting.
        login_counter: 0,
        // false is safe even for an account that IS migrated: `is_lid_migrated()`
        // falls back to the `lid_one_on_one_migration_enabled` ab prop exactly
        // for devices paired before this flag existed.
        lid_migrated: false,
        // 0 = "seed the baseline, do not rotate yet" per the rotation path.
        last_signed_pre_key_rotation_ms: 0,
        // false is WhatsApp's own default (`readreceipts = all`); the real value
        // is refetched from the privacy settings after the next connect.
        read_receipts_disabled: false,
    })
}

/// Carry an opaque value (`Jid`) across the version boundary through its own
/// bincode. The round trip PROVES the two layouts agree instead of assuming it.
fn reserde<A, B>(value: &A, label: &'static str, field: &'static str) -> Result<B, MigrateError>
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

/// Rebuild a Signal keypair from its raw 32-byte halves. Deliberately explicit
/// rather than a serde hop: these bytes ARE the account's identity, so the code
/// that moves them should be readable at a glance.
fn bridge_key_pair(
    old: &wacore06::libsignal::protocol::KeyPair,
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

fn bridge_cert(old: &wacore06::store::CachedNoiseCert) -> wacore::store::CachedNoiseCert {
    wacore::store::CachedNoiseCert {
        key: old.key,
        not_before: old.not_before,
        not_after: old.not_after,
    }
}

/// The one field whose Rust type genuinely changed: `ADVSignedDeviceIdentity`
/// was prost-generated in 0.6 and is buffa-generated in 0.7. The bridge is the
/// PROTOBUF BYTES, which is the stable format; the Rust struct is not.
///
/// Both versions already persist this field as protobuf bytes inside the bincode
/// blob (`account_serde` on either side), so the on-disk shape does not change
/// at all — only the in-memory type does.
fn bridge_account(
    old: &wacore06::store::Device,
    label: &'static str,
) -> Result<
    Option<std::sync::Arc<whatsapp_rust::waproto::whatsapp::ADVSignedDeviceIdentity>>,
    MigrateError,
> {
    let Some(account) = old.account.as_ref() else {
        return Ok(None);
    };
    let bytes = wacore06::store::device::account_serde::to_bytes(account);
    let decoded = wacore::store::device::account_serde::from_bytes(&bytes).map_err(|e| {
        MigrateError::KeyMaterial {
            label,
            field: "account",
            cause: format!("buffa rejected the prost bytes: {e}"),
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
mod blob_migration_tests {
    use super::*;

    /// A 0.6 device with the pairing-time fields populated, encoded the way the
    /// 0.6 daemon would have written it.
    fn legacy_device_blob() -> (wacore06::store::Device, Vec<u8>) {
        let mut device = wacore06::store::Device::new();
        // unwrap: parsing a literal, well-formed JID.
        device.pn = Some("559980000001@s.whatsapp.net".parse().unwrap());
        device.lid = Some("169815004184633@lid".parse().unwrap());
        device.push_name = "wamux migration test".to_string();
        device.next_pre_key_id = 42;
        device.server_has_prekeys = true;
        device.props_hash = Some("abc123".to_string());
        let blob = bincode::serde::encode_to_vec(&device, config()).unwrap();
        (device, blob)
    }

    #[test]
    fn a_legacy_device_blob_does_not_decode_as_0_7() {
        // The premise of the whole migration. If this ever passes, the blobs are
        // compatible and this module is dead code.
        let (_, blob) = legacy_device_blob();
        assert!(decode_whole::<wacore::store::Device>(&blob).is_err());
    }

    #[test]
    fn device_migration_preserves_identity_and_pairing_fields() {
        let (old, blob) = legacy_device_blob();
        let BlobMigration::Rewritten(out) = migrate_device_blob(&blob).unwrap() else {
            panic!("a 0.6 blob must be rewritten, not reported as current");
        };
        let new: wacore::store::Device = decode_whole(&out).unwrap();

        assert_eq!(
            new.pn.as_ref().map(ToString::to_string),
            old.pn.as_ref().map(ToString::to_string)
        );
        assert_eq!(
            new.lid.as_ref().map(ToString::to_string),
            old.lid.as_ref().map(ToString::to_string)
        );
        assert_eq!(new.registration_id, old.registration_id);
        assert_eq!(new.push_name, old.push_name);
        assert_eq!(new.next_pre_key_id, old.next_pre_key_id);
        assert_eq!(new.server_has_prekeys, old.server_has_prekeys);
        assert_eq!(new.props_hash, old.props_hash);
        assert_eq!(new.adv_secret_key, old.adv_secret_key);
        assert_eq!(new.signed_pre_key_signature, old.signed_pre_key_signature);
    }

    // The key material is the account. A silent swap here is indistinguishable
    // from a working migration until the next connect fails to authenticate.
    #[test]
    fn device_migration_carries_every_key_pair_byte_for_byte() {
        let (old, blob) = legacy_device_blob();
        let BlobMigration::Rewritten(out) = migrate_device_blob(&blob).unwrap() else {
            panic!("expected a rewrite");
        };
        let new: wacore::store::Device = decode_whole(&out).unwrap();

        for (label, a, b) in [
            ("noise", &old.noise_key, &new.noise_key),
            ("identity", &old.identity_key, &new.identity_key),
            ("signed_pre", &old.signed_pre_key, &new.signed_pre_key),
        ]
        .map(|(label, a, b)| (label, a, b))
        {
            assert_eq!(
                a.public_key.public_key_bytes(),
                b.public_key.public_key_bytes(),
                "{label} public half changed"
            );
            assert_eq!(
                a.private_key.serialize()[..],
                b.private_key.serialize()[..],
                "{label} private half changed"
            );
        }
    }

    #[test]
    fn the_five_new_device_fields_start_at_their_legacy_defaults() {
        let (_, blob) = legacy_device_blob();
        let BlobMigration::Rewritten(out) = migrate_device_blob(&blob).unwrap() else {
            panic!("expected a rewrite");
        };
        let new: wacore::store::Device = decode_whole(&out).unwrap();
        assert_eq!(new.first_unupload_pre_key_id, 0);
        assert_eq!(new.login_counter, 0);
        assert!(!new.lid_migrated);
        assert_eq!(new.last_signed_pre_key_rotation_ms, 0);
        assert!(!new.read_receipts_disabled);
    }

    // Idempotence is what makes a re-run safe after a partial failure.
    #[test]
    fn re_running_the_device_migration_is_a_no_op() {
        let (_, blob) = legacy_device_blob();
        let BlobMigration::Rewritten(out) = migrate_device_blob(&blob).unwrap() else {
            panic!("expected a rewrite");
        };
        assert_eq!(
            migrate_device_blob(&out).unwrap(),
            BlobMigration::AlreadyCurrent
        );
    }

    #[test]
    fn hash_state_migration_preserves_version_hash_and_map() {
        use std::collections::HashMap;
        let old = wacore06::appstate::hash::HashState {
            version: 49,
            hash: [0xAB; 128],
            index_value_map: HashMap::from([("index-mac".to_string(), vec![1u8, 2, 3])]),
        };
        let blob = bincode::serde::encode_to_vec(&old, config()).unwrap();

        let BlobMigration::Rewritten(out) = migrate_hash_state_blob(&blob).unwrap() else {
            panic!("a 0.6 HashState must be rewritten");
        };
        let new: wacore::appstate::hash::HashState = decode_whole(&out).unwrap();
        assert_eq!(new.version, old.version);
        assert_eq!(new.hash, old.hash);
        assert_eq!(new.index_value_map, old.index_value_map);
        assert!(!new.mac_mismatch_fatal);

        assert_eq!(
            migrate_hash_state_blob(&out).unwrap(),
            BlobMigration::AlreadyCurrent
        );
    }

    // A non-empty index_value_map is the path the live probe could NOT exercise:
    // every HashState in the real store had an empty map.
    #[test]
    fn hash_state_migration_carries_a_populated_index_value_map() {
        use std::collections::HashMap;
        let map: HashMap<String, Vec<u8>> = (0..64)
            .map(|i| (format!("index-{i}"), vec![i as u8; 32]))
            .collect();
        let old = wacore06::appstate::hash::HashState {
            version: 7,
            hash: [0x11; 128],
            index_value_map: map.clone(),
        };
        let blob = bincode::serde::encode_to_vec(&old, config()).unwrap();
        let BlobMigration::Rewritten(out) = migrate_hash_state_blob(&blob).unwrap() else {
            panic!("expected a rewrite");
        };
        let new: wacore::appstate::hash::HashState = decode_whole(&out).unwrap();
        assert_eq!(new.index_value_map, map);
    }

    #[test]
    fn sync_keys_need_no_migration_and_verify_under_both_versions() {
        let key = wacore06::store::traits::AppStateSyncKey {
            key_data: vec![0x11; 32],
            fingerprint: vec![0x22, 0x33, 0x44],
            timestamp: 1_749_400_000,
        };
        let blob = bincode::serde::encode_to_vec(&key, config()).unwrap();
        verify_sync_key_blob(&blob).expect("0.7 must read a 0.6 sync key unchanged");

        let restored: wacore::store::traits::AppStateSyncKey = decode_whole(&blob).unwrap();
        assert_eq!(restored.key_data, key.key_data);
        assert_eq!(restored.fingerprint, key.fingerprint);
        assert_eq!(restored.timestamp, key.timestamp);
    }

    // Garbage must never be "migrated" into a fresh, valid-looking Device.
    #[test]
    fn an_unreadable_blob_is_an_error_not_a_rewrite() {
        let garbage: [u8; 7] = [0xFF, 0x00, 0xDE, 0xAD, 0xBE, 0xEF, 0x01];
        assert!(matches!(
            migrate_device_blob(&garbage),
            Err(MigrateError::Unreadable { .. })
        ));
        assert!(matches!(
            migrate_hash_state_blob(&garbage),
            Err(MigrateError::Unreadable { .. })
        ));
    }
}
