//! Byte formats shared by every storage engine.
//!
//! These are NOT engine details: the blobs must be byte-identical across
//! Postgres and SQLite (and to the whatsapp-rust sqlite reference), so a store
//! dumped from one engine can be loaded by the other. Keeping the codec in one
//! place is what makes that claim testable.

use serde::Serialize;
use serde::de::DeserializeOwned;
use wacore::store::error::StoreError;

/// bincode-standard encode, matching the reference's structured-blob columns
/// (`app_state_keys.key_data`, `app_state_versions.state_data`, `device.data`).
pub fn bincode_encode<T: Serialize>(value: &T) -> Result<Vec<u8>, StoreError> {
    bincode::serde::encode_to_vec(value, bincode::config::standard())
        .map_err(|e| StoreError::Serialization(Box::new(e)))
}

/// bincode-standard decode counterpart.
pub fn bincode_decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, StoreError> {
    bincode::serde::decode_from_slice(bytes, bincode::config::standard())
        .map(|(value, _)| value)
        .map_err(|e| StoreError::Serialization(Box::new(e)))
}

/// Unix seconds, matching the reference's `wacore::time::now_secs()` semantics.
pub fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod blob_format_tests {
    //! Pure round-trip tests (no database) for the structured-blob byte
    //! formats. These blobs must stay drop-in compatible with the
    //! whatsapp-rust sqlite reference backend: a shape or bincode-config
    //! regression here silently corrupts every stored account.

    use std::collections::HashMap;

    use wacore::appstate::hash::HashState;
    use wacore::store::Device;
    use wacore::store::error::StoreError;
    use wacore::store::traits::{AppStateSyncKey, DeviceInfo};
    use whatsapp_rust::Jid;

    use super::{bincode_decode, bincode_encode};

    /// A `Device` with the pairing-time fields populated, like a real account
    /// after QR pairing (a fresh `Device::new()` has them empty).
    fn paired_device_fixture() -> Device {
        let mut device = Device::new();
        // unwrap: parsing a literal, well-formed JID.
        device.pn = Some("559980000001@s.whatsapp.net".parse::<Jid>().unwrap());
        device.push_name = "wamux blob test".to_string();
        device
    }

    #[test]
    fn device_blob_round_trip_preserves_pairing_and_key_material() {
        let device = paired_device_fixture();
        let blob = bincode_encode(&device).unwrap();
        let restored: Device = bincode_decode(&blob).unwrap();

        assert_eq!(restored.pn, device.pn);
        assert_eq!(restored.push_name, device.push_name);
        assert_eq!(restored.registration_id, device.registration_id);
        // Keypairs cross serde via key_pair_serde (priv||pub, 64 bytes); the
        // public halves must survive byte-for-byte or Signal sessions break
        // on the next load. device_props is #[serde(skip)] by design: it
        // decodes to Default and device_store::load() restores DEVICE_PROPS,
        // so it is intentionally not asserted here.
        assert_eq!(
            restored.noise_key.public_key.public_key_bytes(),
            device.noise_key.public_key.public_key_bytes()
        );
        assert_eq!(
            restored.identity_key.public_key.public_key_bytes(),
            device.identity_key.public_key.public_key_bytes()
        );
    }

    #[test]
    fn app_state_sync_key_blob_round_trip_preserves_fields() {
        let key = AppStateSyncKey {
            key_data: vec![0x11; 32],
            fingerprint: vec![0x22, 0x33, 0x44],
            timestamp: 1_749_400_000,
        };
        let blob = bincode_encode(&key).unwrap();
        let restored: AppStateSyncKey = bincode_decode(&blob).unwrap();

        assert_eq!(restored.key_data, key.key_data);
        assert_eq!(restored.fingerprint, key.fingerprint);
        assert_eq!(restored.timestamp, key.timestamp);
    }

    #[test]
    fn hash_state_blob_round_trip_preserves_version_hash_and_map() {
        let state = HashState {
            version: 42,
            // hash is serialized via serde_big_array; a plain [u8; 128] would
            // not derive Serialize, so this exercises that wrapper too.
            hash: [0xAB; 128],
            index_value_map: HashMap::from([("index-mac".to_string(), vec![1u8, 2, 3])]),
            // 0.7 appended this; it is the flag that says the collection's
            // ltHash is beyond repair, so a fresh state starts clean.
            mac_mismatch_fatal: false,
        };
        let blob = bincode_encode(&state).unwrap();
        let restored: HashState = bincode_decode(&blob).unwrap();

        assert_eq!(restored.version, state.version);
        assert_eq!(restored.hash, state.hash);
        assert_eq!(restored.index_value_map, state.index_value_map);
    }

    #[test]
    fn device_registry_json_round_trip_mirrors_protocol_store() {
        // Exactly the encode/decode pair protocol_store.rs uses for
        // device_registry.devices_json: to_string on write, from_str on read.
        // 0.7 added `is_hosted` and the struct-literal form no longer compiles;
        // the constructors are the supported way in and default it to false.
        let devices = vec![
            DeviceInfo::new(0, None),
            DeviceInfo::new(7, Some(3)).with_hosting(true),
        ];
        let json = serde_json::to_string(&devices).unwrap();
        let restored: Vec<DeviceInfo> = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.len(), 2);
        assert_eq!(restored[0].device_id, 0);
        assert_eq!(restored[0].key_index, None);
        assert!(!restored[0].is_hosted);
        assert_eq!(restored[1].device_id, 7);
        assert_eq!(restored[1].key_index, Some(3));
        assert!(restored[1].is_hosted);
    }

    #[test]
    fn device_blob_encoding_is_deterministic() {
        // Two encodes of one Device must be byte-identical. An accidental
        // switch to a non-deterministic bincode config (or a format with
        // unordered maps) would diverge from the sqlite reference blobs.
        let device = paired_device_fixture();
        let first = bincode_encode(&device).unwrap();
        let second = bincode_encode(&device).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn bincode_decode_of_garbage_bytes_returns_serialization_error() {
        // 0xFF is an invalid Option tag in bincode-standard, so this can
        // never accidentally parse; the point is Err, not a panic.
        // (match instead of unwrap_err: Device has no Debug impl.)
        let garbage: [u8; 7] = [0xFF, 0x00, 0xDE, 0xAD, 0xBE, 0xEF, 0x01];
        match bincode_decode::<Device>(&garbage) {
            Err(StoreError::Serialization(_)) => {}
            Err(other) => panic!("expected StoreError::Serialization, got: {other}"),
            Ok(_) => panic!("garbage bytes must not decode into a Device"),
        }
    }
}
