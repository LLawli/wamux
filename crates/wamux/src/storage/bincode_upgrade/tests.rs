//! The contract of the bincode -> protobuf conversion, on blobs built the way a
//! pre-#31 daemon wrote them. No database: the engine halves only read rows and
//! write what this plans.

use std::collections::HashMap;

use wacore::appstate::hash::HashState;
use wacore::store::traits::AppStateSyncKey;

use super::*;
use crate::storage::blob_codec::tests::{every_field_device, persisted_fields};

/// Real blobs written by the 23846f7e build (#86), each next to the protobuf
/// of the same value: see tests/fixtures/bincode-23846f7e/README.md.
macro_rules! fixture_23846f7e {
    ($name:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/bincode-23846f7e/",
            $name
        ))
    };
}
const DEVICE_BINCODE: &[u8] = fixture_23846f7e!("device.bincode");
const DEVICE_PB: &[u8] = fixture_23846f7e!("device.pb");
const HASH_STATE_BINCODE: &[u8] = fixture_23846f7e!("hash_state.bincode");
const HASH_STATE_PB: &[u8] = fixture_23846f7e!("hash_state.pb");
const SYNC_KEY_BINCODE: &[u8] = fixture_23846f7e!("sync_key.bincode");
const SYNC_KEY_PB: &[u8] = fixture_23846f7e!("sync_key.pb");

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

/// The legacy blob is a real one, not `bincode_of(Device)`: bincode is
/// positional, so a blob built from the current `Device` only proves the
/// current layout converts, which is never the layout a legacy store holds.
/// Upstream #1565 added `status_privacy` in the middle of `Device` (#86).
#[test]
fn a_real_23846f7e_device_converts_with_every_field_and_key_intact() {
    let new = upgrade_device_blob(DEVICE_BINCODE, "probe".into()).unwrap();
    let back = decode_device(&new).unwrap();
    let want = decode_device(DEVICE_PB).unwrap();
    assert!(
        persisted_fields(&back) == persisted_fields(&want),
        "a persisted field changed in the conversion"
    );
    assert_eq!(
        back.identity_key.private_key.serialize(),
        want.identity_key.private_key.serialize(),
        "the account identity must survive byte for byte"
    );
    assert_eq!(back.push_name, "wamux blob test");
    assert!(
        back.status_privacy.is_none(),
        "the field did not exist when this blob was written"
    );
}

#[test]
fn a_real_23846f7e_hash_state_converts() {
    let new = upgrade_hash_state_blob(HASH_STATE_BINCODE, "probe".into()).unwrap();
    let back = decode_hash_state(&new).unwrap();
    let want = decode_hash_state(HASH_STATE_PB).unwrap();
    assert_eq!(back.version, want.version);
    assert_eq!(back.hash, want.hash);
    assert_eq!(back.index_value_map, want.index_value_map);
    assert_eq!(back.mac_mismatch_fatal, want.mac_mismatch_fatal);
    assert_eq!(back.bootstrapped, want.bootstrapped);
    assert_eq!(back.version, 7, "the fixture is hash_state(7, true, false)");
}

#[test]
fn a_real_23846f7e_sync_key_converts() {
    let new = upgrade_sync_key_blob(SYNC_KEY_BINCODE, "probe".into()).unwrap();
    let back = decode_app_state_sync_key(&new).unwrap();
    let want = decode_app_state_sync_key(SYNC_KEY_PB).unwrap();
    assert_eq!(back.key_data, want.key_data);
    assert_eq!(back.fingerprint, want.fingerprint);
    assert_eq!(back.timestamp, want.timestamp);
    assert_eq!(back.key_data, sync_key().key_data);
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
        devices: vec![(1, DEVICE_BINCODE.to_vec()), (2, vec![0xFF; 5])],
        versions: vec![],
        sync_keys: vec![],
    };
    let err = plan_bincode_upgrade(rows).unwrap_err();
    assert!(err.to_string().contains("device 2"), "{err}");
}

#[test]
fn the_plan_lists_every_row() {
    let rows = LegacyBlobRows {
        devices: vec![(1, DEVICE_BINCODE.to_vec())],
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
