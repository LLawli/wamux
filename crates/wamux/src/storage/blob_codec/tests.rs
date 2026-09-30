//! The contract of the stored byte formats (#31). Pure: no database. A shape
//! regression here corrupts every stored account, and no DB test would notice,
//! since each engine keeps reading back what it wrote.

use std::collections::HashMap;

use prost::Message as _;
use wacore::appstate::hash::HashState;
use wacore::store::Device;
use wacore::store::device::{CachedNoiseCert, CachedServerCertChain, ServerClientExpiration};
use wacore::store::error::StoreError;
use wacore::store::traits::{AppStateSyncKey, DeviceInfo};
use whatsapp_rust::Jid;

use super::wire::{AppStateSyncKeyWire, DeviceBlob, HashStateWire};
use super::*;

fn cert(seed: u8) -> CachedNoiseCert {
    CachedNoiseCert {
        key: [seed; 32],
        not_before: 1_700_000_000 + i64::from(seed),
        not_after: 1_900_000_000 + i64::from(seed),
    }
}

/// A paired `Device` with every stored field moved off its default, so a field
/// the codec drops or swaps shows up as a mismatch.
pub(crate) fn every_field_device() -> Device {
    let mut device = Device::new();
    // unwrap: parsing literal, well-formed JIDs.
    device.pn = Some("559980000001@s.whatsapp.net".parse::<Jid>().unwrap());
    device.lid = Some("169815004184633:7@lid".parse::<Jid>().unwrap());
    device.signed_pre_key_id = 9;
    device.signed_pre_key_signature = [0x5C; 64];
    device.adv_secret_key = [0xA5; 32];
    device.push_name = "wamux blob test".to_string();
    device.app_version_primary = 2;
    device.app_version_secondary = 3000;
    device.app_version_tertiary = 1_023_456_789;
    device.app_version_last_fetched_ms = 1_758_000_000_000;
    device.edge_routing_info = Some(vec![0x08, 0x02, 0x08, 0x05]);
    device.props_hash = Some("abc123".to_string());
    device.next_pre_key_id = 42;
    device.first_unupload_pre_key_id = 17;
    device.server_has_prekeys = true;
    // Some(empty), not None: the two must stay distinct across a save.
    device.nct_salt = Some(Vec::new());
    device.server_cert_chain = Some(CachedServerCertChain {
        intermediate: cert(1),
        leaf: cert(2),
        signature_verified: true,
    });
    device.login_counter = 5;
    device.lid_migrated = true;
    device.last_signed_pre_key_rotation_ms = 1_757_000_000_000;
    device.server_client_expiration = Some(ServerClientExpiration {
        expires_at: 1_760_000_000,
        version: (2, 3000, 1_023_456_789),
    });
    device.read_receipts_disabled = true;
    device
}

/// Every stored field of two devices, as bincode sees them: Device has no
/// PartialEq, and bincode (the old format) serializes exactly the persisted
/// fields, so equal bytes mean every one of them survived.
pub(crate) fn persisted_fields(device: &Device) -> Vec<u8> {
    bincode::serde::encode_to_vec(device, bincode::config::standard()).unwrap()
}

#[test]
fn device_round_trip_preserves_every_persisted_field() {
    let device = every_field_device();
    let restored = decode_device(&encode_device(&device)).unwrap();
    assert!(
        persisted_fields(&restored) == persisted_fields(&device),
        "a persisted Device field did not survive encode -> decode"
    );
}

#[test]
fn device_key_material_survives_byte_for_byte() {
    // The account's identity. Checked on its own, and on both halves, so a
    // failure names the key instead of "some field differed".
    let device = every_field_device();
    let restored = decode_device(&encode_device(&device)).unwrap();
    for (name, before, after) in [
        ("noise", &device.noise_key, &restored.noise_key),
        ("identity", &device.identity_key, &restored.identity_key),
        (
            "signed_pre",
            &device.signed_pre_key,
            &restored.signed_pre_key,
        ),
    ] {
        assert_eq!(
            before.public_key.public_key_bytes(),
            after.public_key.public_key_bytes(),
            "{name} public half"
        );
        assert_eq!(
            before.private_key.serialize(),
            after.private_key.serialize(),
            "{name} private half"
        );
    }
    assert_eq!(restored.registration_id, device.registration_id);
    assert_eq!(restored.adv_secret_key, device.adv_secret_key);
    assert_eq!(
        restored.signed_pre_key_signature,
        device.signed_pre_key_signature
    );
}

#[test]
fn device_keeps_none_and_some_empty_apart() {
    let mut device = every_field_device();
    device.edge_routing_info = None;
    device.props_hash = Some(String::new());
    let restored = decode_device(&encode_device(&device)).unwrap();
    assert_eq!(restored.edge_routing_info, None);
    assert_eq!(restored.props_hash, Some(String::new()));
    assert_eq!(restored.nct_salt, Some(Vec::new()));
}

#[test]
fn device_jids_come_back_with_their_device_and_server() {
    let device = every_field_device();
    let restored = decode_device(&encode_device(&device)).unwrap();
    let lid = restored.lid.expect("lid");
    assert_eq!(lid.to_string(), "169815004184633:7@lid");
    assert_eq!(lid.device, 7);
    assert_eq!(
        restored.pn.map(|jid| jid.to_string()),
        Some("559980000001@s.whatsapp.net".to_string())
    );
}

#[test]
fn device_runtime_fields_are_restored_not_stored() {
    let mut device = every_field_device();
    device.nct_salt_sync_seen = true;
    let restored = decode_device(&encode_device(&device)).unwrap();
    assert!(
        !restored.nct_salt_sync_seen,
        "runtime-only, never persisted"
    );
    assert_eq!(
        *restored.device_props,
        *wacore::store::device::DEVICE_PROPS,
        "device_props must come back as DEVICE_PROPS, like the reference's load"
    );
}

#[test]
fn a_device_blob_that_predates_a_field_reads_it_as_the_default() {
    // The point of #31. Strip the trailing fields the way an older writer would
    // have left them out, and the rest must still load.
    let mut blob = DeviceBlob::decode(encode_device(&every_field_device()).as_slice()).unwrap();
    blob.server_client_expiration = None;
    blob.read_receipts_disabled = false;
    blob.last_signed_pre_key_rotation_ms = 0;
    let restored = decode_device(&blob.encode_to_vec()).unwrap();
    assert!(restored.server_client_expiration.is_none());
    assert!(!restored.read_receipts_disabled);
    assert_eq!(restored.push_name, "wamux blob test");
}

#[test]
fn device_encoding_is_deterministic() {
    let device = every_field_device();
    assert_eq!(encode_device(&device), encode_device(&device));
}

#[test]
fn device_with_a_short_key_pair_is_rejected() {
    let mut blob = DeviceBlob::decode(encode_device(&every_field_device()).as_slice()).unwrap();
    blob.identity_key.truncate(32);
    match decode_device(&blob.encode_to_vec()) {
        Err(StoreError::Serialization(e)) => assert!(e.to_string().contains("identity_key")),
        Err(other) => panic!("expected a length error, got: {other}"),
        Ok(_) => panic!("a 32-byte key pair must not decode"),
    }
}

#[test]
fn garbage_bytes_are_an_error_not_a_panic() {
    // 0xFF opens a field with wire type 7, which does not exist.
    let garbage: [u8; 7] = [0xFF, 0x00, 0xDE, 0xAD, 0xBE, 0xEF, 0x01];
    assert!(decode_device(&garbage).is_err());
    assert!(decode_hash_state(&garbage).is_err());
    assert!(decode_app_state_sync_key(&garbage).is_err());
}

fn hash_state_fixture() -> HashState {
    HashState {
        version: 42,
        hash: [0xAB; 128],
        index_value_map: HashMap::from([
            ("index-mac-b".to_string(), vec![4u8, 5]),
            ("index-mac-a".to_string(), vec![1u8, 2, 3]),
            ("index-mac-c".to_string(), vec![]),
        ]),
        mac_mismatch_fatal: true,
        bootstrapped: true,
    }
}

#[test]
fn hash_state_round_trip_preserves_every_field() {
    let state = hash_state_fixture();
    let restored = decode_hash_state(&encode_hash_state(&state)).unwrap();
    assert_eq!(restored.version, state.version);
    assert_eq!(restored.hash, state.hash);
    assert_eq!(restored.index_value_map, state.index_value_map);
    assert!(
        restored.mac_mismatch_fatal,
        "a latched collection stays latched"
    );
    assert!(restored.bootstrapped);
}

#[test]
fn hash_state_encoding_does_not_depend_on_map_order() {
    // Two HashMaps with the same entries iterate in different orders (each has
    // its own random seed). The bytes must not, or the engines' blobs diverge.
    let first = encode_hash_state(&hash_state_fixture());
    for _ in 0..16 {
        assert_eq!(encode_hash_state(&hash_state_fixture()), first);
    }
}

#[test]
fn hash_state_that_predates_the_flags_reads_as_healthy_and_unbootstrapped() {
    let bytes = HashStateWire {
        version: 9,
        hash: vec![0u8; 128],
        ..Default::default()
    }
    .encode_to_vec();
    let restored = decode_hash_state(&bytes).unwrap();
    assert_eq!(restored.version, 9);
    assert!(!restored.mac_mismatch_fatal);
    assert!(!restored.bootstrapped);
}

#[test]
fn hash_state_with_a_short_hash_is_rejected() {
    let bytes = HashStateWire {
        version: 1,
        hash: vec![0u8; 64],
        ..Default::default()
    }
    .encode_to_vec();
    assert!(decode_hash_state(&bytes).is_err());
}

#[test]
fn sync_key_round_trip_preserves_every_field() {
    let key = AppStateSyncKey {
        key_data: vec![0x11; 32],
        fingerprint: vec![0x22, 0x33, 0x44],
        timestamp: 1_749_400_000,
    };
    let restored = decode_app_state_sync_key(&encode_app_state_sync_key(&key)).unwrap();
    assert_eq!(restored.key_data, key.key_data);
    assert_eq!(restored.fingerprint, key.fingerprint);
    assert_eq!(restored.timestamp, key.timestamp);
}

#[test]
fn sync_key_that_is_not_32_bytes_is_rejected() {
    let bytes = AppStateSyncKeyWire {
        key_data: vec![0u8; 16],
        fingerprint: vec![1],
        timestamp: 1,
    }
    .encode_to_vec();
    assert!(decode_app_state_sync_key(&bytes).is_err());
}

#[test]
fn sync_key_bytes_are_pinned() {
    // Golden bytes: the field numbers ARE the stored format. A renumbered field
    // still round-trips (both sides move together) and passes every test above;
    // only this catches it. Also the upstream sqlite backend's exact bytes for
    // this value, which is what makes the rows interchangeable.
    let key = AppStateSyncKey {
        key_data: vec![0x11; 32],
        fingerprint: vec![0x22],
        timestamp: 1,
    };
    let mut expected = vec![0x0A, 32];
    expected.extend([0x11; 32]);
    expected.extend([0x12, 1, 0x22, 0x18, 1]);
    assert_eq!(encode_app_state_sync_key(&key), expected);
}

#[test]
fn device_field_numbers_are_pinned() {
    // Same reason as sync_key_bytes_are_pinned, for the one message with no
    // upstream twin. Only the scalar tail is pinned: key material is random.
    let blob = DeviceBlob {
        registration_id: 1,
        login_counter: 2,
        read_receipts_disabled: true,
        ..Default::default()
    };
    // field 3 varint 1 | field 23 varint 2 | field 27 varint 1
    assert_eq!(
        blob.encode_to_vec(),
        vec![0x18, 1, 0xB8, 0x01, 2, 0xD8, 0x01, 1]
    );
}

#[test]
fn device_registry_json_round_trip_mirrors_protocol_store() {
    // Exactly the encode/decode pair protocol_store.rs uses for
    // device_registry.devices_json: to_string on write, from_str on read. Not a
    // blob_codec format, but the one other structured value on disk, pinned
    // here with the rest.
    let devices = vec![
        DeviceInfo::new(0, None),
        DeviceInfo::new(7, Some(3)).with_hosting(true),
    ];
    let json = serde_json::to_string(&devices).unwrap();
    let restored: Vec<DeviceInfo> = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.len(), 2);
    assert_eq!(restored[1].device_id(), 7);
    assert_eq!(restored[1].key_index(), Some(3));
    assert!(restored[1].is_hosted());
    assert_eq!(
        json,
        r#"[{"device_id":0,"key_index":null,"is_hosted":false},{"device_id":7,"key_index":3,"is_hosted":true}]"#,
        "the stored devices_json shape must not change"
    );
}
