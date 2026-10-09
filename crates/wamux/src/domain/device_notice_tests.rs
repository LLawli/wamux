//! #151 (part 4 of #141): someone's devices or identity key, routed through
//! `map_event`, so each test also proves the event no longer falls into the
//! RawEvent catch-all. The add is the one the production subscription carried
//! on 2026-10-09 at 12:29Z, the remove the one #135 captured (jids anonymized,
//! the signed bytes replaced by a stand-in of the same role). Both measured
//! `key_index.timestamp` in seconds.

use std::str::FromStr;

use wacore::stanza::devices::KeyIndexInfo;
use wacore::types::events::{
    DeviceListUpdate, DeviceListUpdateType, DeviceNotificationInfo, Event, IdentityChange,
};
use whatsapp_rust::Jid;

use crate::domain::event_mapping::map_event;
use crate::proto::v1 as pb;
use crate::proto::v1::device_notice::Notice;
use crate::proto::v1::event_envelope::Event as PbEvent;

const USER: &str = "100000000000003@lid";
const USER_PN: &str = "5511900000003@s.whatsapp.net";
const SIGNED: &[u8] = &[10, 22, 8, 165, 174, 142, 183, 2, 16, 240, 18, 64, 49, 21];

fn lib(value: &str) -> Jid {
    Jid::from_str(value).expect("test jid")
}

fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

/// The one DeviceNotice the event maps to; anything else is a failure.
fn notice_of(event: Event) -> Notice {
    match map_event(&event).as_slice() {
        [PbEvent::DeviceNotice(pb::DeviceNotice { notice: Some(n) })] => n.clone(),
        other => panic!("expected one DeviceNotice, got {other:?}"),
    }
}

fn device(device_id: u32, key_index: Option<u32>) -> DeviceNotificationInfo {
    DeviceNotificationInfo::builder()
        .device_id(device_id)
        .maybe_key_index(key_index)
        .build()
}

#[test]
fn device_list_relays_the_captured_add() {
    let event = Event::DeviceListUpdate(
        DeviceListUpdate::builder()
            .user(lib(USER))
            .update_type(DeviceListUpdateType::Add)
            .devices(vec![device(43, Some(7))])
            .key_index(KeyIndexInfo {
                timestamp: 1_791_548_912,
                signed_bytes: Some(SIGNED.to_vec()),
            })
            .build(),
    );
    let expected = Notice::DeviceList(pb::DeviceListChange {
        user: wire(USER),
        lid_user: None,
        update_type: pb::DeviceListChangeType::Add as i32,
        devices: vec![pb::DeviceKeyIndex {
            device_id: 43,
            key_index: Some(7),
        }],
        key_index: Some(pb::KeyIndexList {
            timestamp: 1_791_548_912_000,
            signed_bytes: Some(SIGNED.to_vec()),
        }),
        contact_hash: None,
    });
    assert_eq!(notice_of(event), expected);
}

#[test]
fn device_list_relays_the_captured_remove() {
    let event = Event::DeviceListUpdate(
        DeviceListUpdate::builder()
            .user(lib(USER))
            .update_type(DeviceListUpdateType::Remove)
            .devices(vec![device(23, Some(1))])
            .key_index(KeyIndexInfo {
                timestamp: 1_790_867_950,
                signed_bytes: None,
            })
            .build(),
    );
    match notice_of(event) {
        Notice::DeviceList(change) => {
            assert_eq!(change.update_type, pb::DeviceListChangeType::Remove as i32);
            assert_eq!(
                change.key_index,
                Some(pb::KeyIndexList {
                    timestamp: 1_790_867_950_000,
                    signed_bytes: None,
                })
            );
        }
        other => panic!("expected device_list, got {other:?}"),
    }
}

/// An update carries the contact hash and the LID, and no key index.
#[test]
fn device_list_update_carries_the_hash_and_the_lid() {
    let event = Event::DeviceListUpdate(
        DeviceListUpdate::builder()
            .user(lib(USER_PN))
            .lid_user(lib(USER))
            .update_type(DeviceListUpdateType::Update)
            .devices(vec![device(2, None)])
            .contact_hash("aGFzaA==".to_string())
            .build(),
    );
    let expected = Notice::DeviceList(pb::DeviceListChange {
        user: wire(USER_PN),
        lid_user: wire(USER),
        update_type: pb::DeviceListChangeType::Update as i32,
        devices: vec![pb::DeviceKeyIndex {
            device_id: 2,
            key_index: None,
        }],
        key_index: None,
        contact_hash: Some("aGFzaA==".to_string()),
    });
    assert_eq!(notice_of(event), expected);
}

/// A nonsense key-index time saturates instead of overflowing.
#[test]
fn device_list_key_index_time_saturates() {
    let event = Event::DeviceListUpdate(
        DeviceListUpdate::builder()
            .user(lib(USER))
            .update_type(DeviceListUpdateType::Remove)
            .devices(Vec::new())
            .key_index(KeyIndexInfo {
                timestamp: i64::MAX,
                signed_bytes: None,
            })
            .build(),
    );
    match notice_of(event) {
        Notice::DeviceList(change) => {
            assert_eq!(change.key_index.map(|k| k.timestamp), Some(i64::MAX));
        }
        other => panic!("expected device_list, got {other:?}"),
    }
}

#[test]
fn identity_change_relays_user_lid_and_implicit() {
    for implicit in [true, false] {
        let event = Event::IdentityChange(
            IdentityChange::builder()
                .user(lib(USER_PN))
                .lid_user(lib(USER))
                .implicit(implicit)
                .build(),
        );
        let expected = Notice::Identity(pb::IdentityChangeInfo {
            user: wire(USER_PN),
            lid_user: wire(USER),
            implicit,
        });
        assert_eq!(notice_of(event), expected);
    }
}
