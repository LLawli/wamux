//! The contract of the bincode -> protobuf conversion, on blobs built the way a
//! pre-#31 daemon wrote them. No database: the engine halves only read rows and
//! write what this plans.

use std::collections::HashMap;

use wacore::appstate::hash::HashState;
use wacore::store::traits::AppStateSyncKey;

use super::*;
use crate::storage::blob_codec::tests::{every_field_device, persisted_fields};

fn bincode_of<T: serde::Serialize>(value: &T) -> Vec<u8> {
    bincode::serde::encode_to_vec(value, bincode::config::standard()).unwrap()
}

fn hash_state(version: u64, bootstrapped: bool, mac_mismatch_fatal: bool) -> HashState {
    HashState {
        version,
        hash: [0x7E; 128],
        index_value_map: HashMap::from([
            ("a".to_string(), vec![1u8]),
            ("b".to_string(), vec![2u8, 3]),
        ]),
        mac_mismatch_fatal,
        bootstrapped,
    }
}

fn sync_key() -> AppStateSyncKey {
    AppStateSyncKey {
        key_data: vec![0x42; 32],
        fingerprint: vec![1, 2, 3],
        timestamp: 1_749_400_000,
    }
}

#[test]
fn a_legacy_device_converts_with_every_persisted_field_and_key_intact() {
    let device = every_field_device();
    let new = upgrade_device_blob(&bincode_of(&device), "probe".into()).unwrap();
    let back = decode_device(&new).unwrap();
    assert!(persisted_fields(&back) == persisted_fields(&device));
    assert_eq!(
        back.identity_key.private_key.serialize(),
        device.identity_key.private_key.serialize(),
        "the account identity must survive byte for byte"
    );
}

#[test]
fn a_blob_that_is_not_legacy_bincode_is_refused_not_converted() {
    // An already-protobuf blob under a 'bincode' marker is the case that must
    // never be "converted": it would be destroyed.
    let protobuf = encode_device(&every_field_device());
    let err = upgrade_device_blob(&protobuf, "device.data of device 3".into()).unwrap_err();
    assert!(
        matches!(err, BincodeUpgradeError::Unreadable { .. }),
        "{err}"
    );
    assert!(
        err.to_string().contains("device 3"),
        "the error names the row"
    );
}

#[test]
fn a_legacy_blob_with_trailing_bytes_is_refused() {
    let mut blob = bincode_of(&sync_key());
    blob.push(0);
    let err = upgrade_sync_key_blob(&blob, "probe".into()).unwrap_err();
    assert!(err.to_string().contains("trailing"), "{err}");
}

/// Every field is carried over as it was, `bootstrapped` included: the
/// conversion no longer marks an unmarked synced collection (#36), the
/// library's first sync at the head does. `mac_mismatch_fatal` is a real latch.
#[test]
fn a_legacy_hash_state_converts_field_for_field() {
    let shapes = [
        hash_state(1399, false, true),
        hash_state(0, false, false),
        hash_state(339, true, false),
    ];
    for old in shapes {
        let new = upgrade_hash_state_blob(&bincode_of(&old), "probe".into()).unwrap();
        let back = decode_hash_state(&new).unwrap();
        assert_eq!(back.version, old.version);
        assert_eq!(back.bootstrapped, old.bootstrapped, "v{}", old.version);
        assert_eq!(back.mac_mismatch_fatal, old.mac_mismatch_fatal);
        assert_eq!(back.hash, old.hash);
        assert_eq!(back.index_value_map, old.index_value_map);
    }
}

#[test]
fn a_legacy_sync_key_converts() {
    let new = upgrade_sync_key_blob(&bincode_of(&sync_key()), "probe".into()).unwrap();
    let back = decode_app_state_sync_key(&new).unwrap();
    assert_eq!(back.key_data, sync_key().key_data);
    assert_eq!(back.fingerprint, sync_key().fingerprint);
    assert_eq!(back.timestamp, sync_key().timestamp);
}

#[test]
fn one_bad_row_fails_the_whole_plan() {
    let rows = LegacyBlobRows {
        devices: vec![(1, bincode_of(&every_field_device())), (2, vec![0xFF; 5])],
        versions: vec![],
        sync_keys: vec![],
    };
    let err = plan_bincode_upgrade(rows).unwrap_err();
    assert!(err.to_string().contains("device 2"), "{err}");
}

#[test]
fn the_plan_lists_every_row() {
    let rows = LegacyBlobRows {
        devices: vec![(1, bincode_of(&every_field_device()))],
        versions: vec![
            (
                1,
                "regular_low".into(),
                bincode_of(&hash_state(1399, false, true)),
            ),
            (
                1,
                "critical_unblock_low".into(),
                bincode_of(&hash_state(12, true, false)),
            ),
            (
                1,
                "regular".into(),
                bincode_of(&hash_state(0, false, false)),
            ),
        ],
        sync_keys: vec![(1, vec![0, 0, 1], bincode_of(&sync_key()))],
    };
    let plan = plan_bincode_upgrade(rows).unwrap();
    assert_eq!(plan.devices.len(), 1);
    assert_eq!(plan.versions.len(), 3);
    assert_eq!(plan.sync_keys.len(), 1);
}

#[test]
fn the_marker_is_read_strictly() {
    assert!(needs_bincode_upgrade(BLOB_FORMAT_BINCODE).unwrap());
    assert!(!needs_bincode_upgrade(BLOB_FORMAT_PROTOBUF).unwrap());
    assert!(matches!(
        needs_bincode_upgrade("json"),
        Err(BincodeUpgradeError::UnknownFormat(_))
    ));
}
