//! `Device.status_privacy` in the stored blob (#86, upstream #1565): the status
//! audience app-state sync delivers. Kept whole, as the library's own bytes,
//! because an unknown mode or a dropped list would widen who sees a status.

use std::sync::Arc;

use prost::Message as _;
use wacore::store::device::status_privacy_serde;
use whatsapp_rust::buffa::EnumValue;
use whatsapp_rust::waproto::whatsapp::sync_action_value::StatusPrivacyAction;
use whatsapp_rust::waproto::whatsapp::sync_action_value::status_privacy_action::CustomList;

use super::tests::{every_field_device, status_privacy_action};
use super::wire::DeviceBlob;
use super::*;

#[test]
fn device_status_privacy_is_stored_as_the_librarys_own_bytes() {
    let blob = DeviceBlob::decode(encode_device(&every_field_device()).as_slice()).unwrap();
    assert_eq!(
        blob.status_privacy,
        Some(status_privacy_serde::to_bytes(&status_privacy_action()))
    );
}

#[test]
fn device_status_privacy_keeps_unknown_modes_and_lists() {
    // Modes this build has no name for must come back as the same numbers,
    // not as absent or as a default a consumer would read as "all contacts".
    let action = StatusPrivacyAction {
        mode: Some(EnumValue::Unknown(99)),
        modes: vec![EnumValue::Unknown(100)],
        user_jid: vec!["120363000000000042@lid".into()],
        custom_lists: vec![CustomList {
            list_id: Some("close".into()),
            user_jid: vec!["120363000000000044@lid".into()],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut device = every_field_device();
    device.status_privacy = Some(Arc::new(action.clone()));
    let restored = decode_device(&encode_device(&device)).unwrap();
    assert_eq!(restored.status_privacy.as_deref(), Some(&action));
}

#[test]
fn device_status_privacy_absent_stays_none() {
    let mut device = every_field_device();
    device.status_privacy = None;
    let bytes = encode_device(&device);
    assert_eq!(
        DeviceBlob::decode(bytes.as_slice()).unwrap().status_privacy,
        None
    );
    assert!(decode_device(&bytes).unwrap().status_privacy.is_none());
}

#[test]
fn device_status_privacy_that_does_not_decode_is_none_not_an_error() {
    // An optional field must not keep the account from loading, as upstream's
    // own store does: an audience that does not decode is unknown.
    for (what, bad) in [
        ("truncated protobuf", vec![0x0a, 0xff]),
        ("no mode", Vec::new()),
    ] {
        let mut blob = DeviceBlob::decode(encode_device(&every_field_device()).as_slice()).unwrap();
        blob.status_privacy = Some(bad);
        let restored =
            decode_device(&blob.encode_to_vec()).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(restored.status_privacy.is_none(), "{what}");
        assert_eq!(
            restored.push_name, "wamux blob test",
            "{what}: the rest loads"
        );
    }
}

#[test]
fn device_status_privacy_field_number_is_pinned() {
    // field 28, wire type 2: tag (28 << 3) | 2 = 226, varint E2 01, then length.
    let blob = DeviceBlob {
        status_privacy: Some(vec![0x08, 0x01]),
        ..Default::default()
    };
    assert_eq!(blob.encode_to_vec(), vec![0xE2, 0x01, 2, 0x08, 0x01]);
}
