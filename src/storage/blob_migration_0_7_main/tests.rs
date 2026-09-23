//! The contract of the 0.7.0 -> main blob migration (#30). Fixtures are built
//! with the RELEASED types (`wacore070`) and encoded the way a 0.7.0 daemon
//! wrote them; the result is read back with the git `wacore`.

use super::*;
use crate::storage::blob_migration::decode_whole;

type Old = wacore070::store::Device;
type New = wacore::store::Device;

fn old_cert(seed: u8) -> wacore070::store::CachedNoiseCert {
    wacore070::store::CachedNoiseCert {
        key: [seed; 32],
        not_before: 1_700_000_000 + i64::from(seed),
        not_after: 1_900_000_000 + i64::from(seed),
    }
}

/// A 0.7.0 device with every field that is not a default set to something
/// recognisable, so a field dropped or shifted by the bridge shows up.
fn released_device_blob() -> (Old, Vec<u8>) {
    let mut device = Old::new();
    // unwrap: parsing literal, well-formed JIDs.
    device.pn = Some("559980000001@s.whatsapp.net".parse().unwrap());
    device.lid = Some("169815004184633@lid".parse().unwrap());
    device.push_name = "wamux migration test".to_string();
    device.app_version_last_fetched_ms = 1_758_000_000_000;
    device.edge_routing_info = Some(vec![0x08, 0x02, 0x08, 0x05]);
    device.props_hash = Some("abc123".to_string());
    device.next_pre_key_id = 42;
    device.first_unupload_pre_key_id = 17;
    device.server_has_prekeys = true;
    device.nct_salt = Some(vec![0x5A; 16]);
    device.server_cert_chain = Some(wacore070::store::CachedServerCertChain {
        intermediate: old_cert(1),
        leaf: old_cert(2),
    });
    device.login_counter = 5;
    device.lid_migrated = true;
    device.last_signed_pre_key_rotation_ms = 1_757_000_000_000;
    device.read_receipts_disabled = true;
    let blob = bincode::serde::encode_to_vec(&device, bincode::config::standard()).unwrap();
    (device, blob)
}

fn migrated(blob: &[u8]) -> New {
    let BlobMigration::Rewritten(out) = migrate_device_blob(blob).unwrap() else {
        panic!("a 0.7.0 device blob must be rewritten, not reported as current");
    };
    decode_whole(&out).unwrap()
}

fn jid_string<J: ToString>(jid: &Option<J>) -> Option<String> {
    jid.as_ref().map(ToString::to_string)
}

#[test]
fn a_released_device_blob_does_not_decode_as_main() {
    // The premise of the whole migration. If this ever passes, the layouts
    // agree and this module is dead code.
    let (_, blob) = released_device_blob();
    assert!(decode_whole::<New>(&blob).is_err());
}

#[test]
fn device_migration_preserves_identity_and_pairing_fields() {
    let (old, blob) = released_device_blob();
    let new = migrated(&blob);

    assert_eq!(jid_string(&new.pn), jid_string(&old.pn));
    assert_eq!(jid_string(&new.lid), jid_string(&old.lid));
    assert_eq!(new.registration_id, old.registration_id);
    assert_eq!(new.signed_pre_key_id, old.signed_pre_key_id);
    assert_eq!(new.signed_pre_key_signature, old.signed_pre_key_signature);
    assert_eq!(new.adv_secret_key, old.adv_secret_key);
    assert_eq!(new.push_name, old.push_name);
    assert_eq!(new.app_version_primary, old.app_version_primary);
    assert_eq!(new.app_version_secondary, old.app_version_secondary);
    assert_eq!(new.app_version_tertiary, old.app_version_tertiary);
    assert_eq!(
        new.app_version_last_fetched_ms,
        old.app_version_last_fetched_ms
    );
    assert_eq!(new.edge_routing_info, old.edge_routing_info);
    assert_eq!(new.props_hash, old.props_hash);
    assert_eq!(new.next_pre_key_id, old.next_pre_key_id);
    assert_eq!(new.server_has_prekeys, old.server_has_prekeys);
    assert_eq!(new.nct_salt, old.nct_salt);
}

// The fields 0.7.0 itself added over 0.6. A bridge written against the 0.6
// migration would reset them to their "legacy" defaults, which on a real 0.7.0
// store is data loss (lid_migrated, the prekey watermark, the rotation clock).
#[test]
fn device_migration_carries_the_fields_0_7_0_already_had() {
    let (old, blob) = released_device_blob();
    let new = migrated(&blob);

    assert_eq!(new.first_unupload_pre_key_id, old.first_unupload_pre_key_id);
    assert_eq!(new.login_counter, old.login_counter);
    assert_eq!(new.lid_migrated, old.lid_migrated);
    assert_eq!(
        new.last_signed_pre_key_rotation_ms,
        old.last_signed_pre_key_rotation_ms
    );
    assert_eq!(new.read_receipts_disabled, old.read_receipts_disabled);
}

// The key material is the account. A silent swap here is indistinguishable
// from a working migration until the next connect fails to authenticate.
#[test]
fn device_migration_carries_every_key_pair_byte_for_byte() {
    let (old, blob) = released_device_blob();
    let new = migrated(&blob);

    let pairs = [
        ("noise", &old.noise_key, &new.noise_key),
        ("identity", &old.identity_key, &new.identity_key),
        ("signed_pre", &old.signed_pre_key, &new.signed_pre_key),
    ];
    for (label, a, b) in pairs {
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
fn a_cached_cert_chain_survives_but_is_marked_unverified() {
    let (old, blob) = released_device_blob();
    let new = migrated(&blob);

    let old_chain = old.server_cert_chain.as_ref().unwrap();
    let chain = new
        .server_cert_chain
        .as_ref()
        .expect("the cached chain must be carried, not dropped");
    for (label, a, b) in [
        ("intermediate", &old_chain.intermediate, &chain.intermediate),
        ("leaf", &old_chain.leaf, &chain.leaf),
    ] {
        assert_eq!(a.key, b.key, "{label} key changed");
        assert_eq!(a.not_before, b.not_before, "{label} not_before changed");
        assert_eq!(a.not_after, b.not_after, "{label} not_after changed");
    }
    // Upstream's rule for a legacy record: untrusted until one XX re-verifies it.
    assert!(!chain.signature_verified);
}

#[test]
fn a_device_without_a_cached_chain_stays_without_one() {
    let (mut old, _) = released_device_blob();
    old.server_cert_chain = None;
    let blob = bincode::serde::encode_to_vec(&old, bincode::config::standard()).unwrap();
    assert!(migrated(&blob).server_cert_chain.is_none());
}

#[test]
fn server_client_expiration_starts_absent() {
    let (_, blob) = released_device_blob();
    assert!(migrated(&blob).server_client_expiration.is_none());
}

// Idempotence is what makes a re-run safe after a partial failure.
#[test]
fn re_running_the_device_migration_is_a_no_op() {
    let (_, blob) = released_device_blob();
    let BlobMigration::Rewritten(out) = migrate_device_blob(&blob).unwrap() else {
        panic!("expected a rewrite");
    };
    assert_eq!(
        migrate_device_blob(&out).unwrap(),
        BlobMigration::AlreadyCurrent
    );
}

#[test]
fn a_device_written_by_main_is_already_current() {
    let device = New::new();
    let blob = bincode::serde::encode_to_vec(&device, bincode::config::standard()).unwrap();
    assert_eq!(
        migrate_device_blob(&blob).unwrap(),
        BlobMigration::AlreadyCurrent
    );
}

fn released_hash_state(map_entries: u8) -> (wacore070::appstate::hash::HashState, Vec<u8>) {
    let old = wacore070::appstate::hash::HashState {
        version: 49,
        hash: [0xAB; 128],
        index_value_map: (0..map_entries)
            .map(|i| (format!("index-{i}"), vec![i; 32]))
            .collect(),
        mac_mismatch_fatal: true,
    };
    let blob = bincode::serde::encode_to_vec(&old, bincode::config::standard()).unwrap();
    (old, blob)
}

#[test]
fn a_released_hash_state_blob_does_not_decode_as_main() {
    let (_, blob) = released_hash_state(0);
    assert!(decode_whole::<wacore::appstate::hash::HashState>(&blob).is_err());
}

#[test]
fn hash_state_migration_preserves_every_old_field() {
    // 64 entries: the live 0.6 probe found only empty maps, so a populated
    // map is the path no production run has exercised.
    let (old, blob) = released_hash_state(64);
    let BlobMigration::Rewritten(out) = migrate_hash_state_blob(&blob).unwrap() else {
        panic!("a 0.7.0 HashState must be rewritten");
    };
    let new: wacore::appstate::hash::HashState = decode_whole(&out).unwrap();
    assert_eq!(new.version, old.version);
    assert_eq!(new.hash, old.hash);
    assert_eq!(new.index_value_map, old.index_value_map);
    assert_eq!(new.mac_mismatch_fatal, old.mac_mismatch_fatal);
}

#[test]
fn a_migrated_hash_state_is_not_bootstrapped() {
    let (_, blob) = released_hash_state(1);
    let BlobMigration::Rewritten(out) = migrate_hash_state_blob(&blob).unwrap() else {
        panic!("expected a rewrite");
    };
    let new: wacore::appstate::hash::HashState = decode_whole(&out).unwrap();
    assert!(!new.bootstrapped);
    assert_eq!(
        migrate_hash_state_blob(&out).unwrap(),
        BlobMigration::AlreadyCurrent
    );
}

#[test]
fn sync_keys_need_no_migration_and_verify_under_main() {
    let key = wacore070::store::traits::AppStateSyncKey {
        key_data: vec![0x11; 32],
        fingerprint: vec![0x22, 0x33, 0x44],
        timestamp: 1_749_400_000,
    };
    let blob = bincode::serde::encode_to_vec(&key, bincode::config::standard()).unwrap();
    verify_sync_key_blob(&blob).expect("main must read a 0.7.0 sync key unchanged");
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
    assert!(matches!(
        verify_sync_key_blob(&garbage),
        Err(MigrateError::Unreadable { .. })
    ));
}
