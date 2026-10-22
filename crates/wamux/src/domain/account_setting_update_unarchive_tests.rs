//! #215: the "keep chats archived" setting, routed through `map_event`, so each
//! test also proves the event does not fall into the RawEvent catch-all. Hand
//! built from the library's type: no capture of this mutation exists yet.

#![allow(clippy::field_reassign_with_default)] // the generated action is built by assignment

use wacore::types::events::{Event, UnarchiveChatsSettingUpdate};
use whatsapp_rust::waproto::whatsapp::sync_action_value::UnarchiveChatsSetting;

use crate::domain::event_mapping::map_event;
use crate::proto::v1 as pb;
use crate::proto::v1::account_setting_update::Setting;
use crate::proto::v1::event_envelope::Event as PbEvent;

/// The instant `ms` after the epoch, as the library's builders take it.
macro_rules! at_ms {
    ($ms:expr) => {
        wacore::time::from_millis($ms).expect("test instant")
    };
}

fn unarchive_event(unarchive_chats: bool, ms: i64, from_full_sync: bool) -> Event {
    let mut action = UnarchiveChatsSetting::default();
    action.unarchive_chats = Some(unarchive_chats);
    Event::UnarchiveChatsSettingUpdate(
        UnarchiveChatsSettingUpdate::builder()
            .unarchive_chats(unarchive_chats)
            .timestamp(at_ms!(ms))
            .action(Box::new(action))
            .from_full_sync(from_full_sync)
            .build(),
    )
}

fn setting_of(event: Event) -> pb::AccountSettingUpdate {
    match map_event(&event).as_slice() {
        [PbEvent::AccountSetting(update)] => update.clone(),
        other => panic!("expected one AccountSettingUpdate, got {other:?}"),
    }
}

#[test]
fn unarchive_chats_true_is_relayed_as_its_own_setting() {
    let update = setting_of(unarchive_event(true, 1_792_000_000_123, false));
    assert_eq!(update.timestamp, 1_792_000_000_123);
    assert_eq!(
        update.action_timestamp, None,
        "the library keeps no mutation time here"
    );
    assert!(!update.from_full_sync);
    assert_eq!(
        update.setting,
        Some(Setting::UnarchiveChats(pb::UnarchiveChatsSetting {
            unarchive_chats: true
        }))
    );
}

#[test]
fn unarchive_chats_false_is_relayed_not_dropped() {
    let update = setting_of(unarchive_event(false, 1_792_000_000_456, true));
    assert_eq!(update.timestamp, 1_792_000_000_456);
    assert!(update.from_full_sync);
    assert_eq!(
        update.setting,
        Some(Setting::UnarchiveChats(pb::UnarchiveChatsSetting {
            unarchive_chats: false
        })),
        "false means \"keep chats archived\" is on, a value the edge needs"
    );
}
