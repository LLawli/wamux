//! #148 (part 1 of #141): four more chat mutations, asserted as the whole
//! message and routed through `map_event`, so each test also proves the event
//! no longer falls into the RawEvent catch-all. Values are the ones the user's
//! phone sent on 2026-10-08 14:43Z to 15:25Z (read off the production
//! subscription; jids anonymized, message ids as sent). The participant and the
//! absent flags are hand-built: no capture had them.

#![allow(clippy::field_reassign_with_default)] // the generated actions are built by assignment

use std::str::FromStr;

use wacore::types::events::{
    ClearChatUpdate, DeleteMessageForMeUpdate, Event, LockChatUpdate, UserStatusMuteUpdate,
};
use whatsapp_rust::Jid;
use whatsapp_rust::waproto::whatsapp::MessageKey;
use whatsapp_rust::waproto::whatsapp::sync_action_value::{
    ClearChatAction, DeleteMessageForMeAction, LockChatAction, SyncActionMessage,
    SyncActionMessageRange, UserStatusMuteAction,
};

use crate::domain::event_mapping::map_event;
use crate::proto::v1 as pb;
use crate::proto::v1::app_state_update::Action;
use crate::proto::v1::event_envelope::Event as PbEvent;

const CHAT: &str = "100000000000148@lid";
const GROUP: &str = "120363000000000148@g.us";
const SENDER: &str = "100000000000003@lid";
const CONTACT: &str = "5511900000148@s.whatsapp.net";

/// The instant `ms` after the epoch, as the library's builders take it. A
/// macro because wamux does not depend on chrono and cannot name the type.
macro_rules! at_ms {
    ($ms:expr) => {
        wacore::time::from_millis($ms).expect("test instant")
    };
}

fn lib(value: &str) -> Jid {
    Jid::from_str(value).expect("test jid")
}

fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

/// The one AppStateUpdate the event maps to; anything else is a failure.
fn app_state_of(event: Event) -> pb::AppStateUpdate {
    match map_event(&event).as_slice() {
        [PbEvent::AppState(update)] => update.clone(),
        other => panic!("expected one AppStateUpdate, got {other:?}"),
    }
}

/// The header the update carries, with the action's time and its case.
fn header(chat: &str, ms: i64, action: Action) -> pb::AppStateUpdate {
    pb::AppStateUpdate {
        chat: wire(chat),
        timestamp: ms,
        action_timestamp: Some(ms),
        from_full_sync: false,
        action: Some(action),
    }
}

fn lock_event(locked: Option<bool>) -> Event {
    let mut action = LockChatAction::default();
    action.locked = locked;
    Event::LockChatUpdate(
        LockChatUpdate::builder()
            .jid(lib(CHAT))
            .timestamp(at_ms!(1_791_470_632_956))
            .action_timestamp(at_ms!(1_791_470_632_956))
            .action(Box::new(action))
            .from_full_sync(false)
            .build(),
    )
}

#[test]
fn lock_chat_update_relays_locked_and_the_chat() {
    let expected = header(
        CHAT,
        1_791_470_632_956,
        Action::Lock(pb::LockChange { locked: Some(true) }),
    );
    assert_eq!(app_state_of(lock_event(Some(true))), expected);
}

#[test]
fn lock_chat_update_without_the_flag_leaves_locked_unset() {
    let update = app_state_of(lock_event(None));
    assert_eq!(
        update.action,
        Some(Action::Lock(pb::LockChange { locked: None }))
    );
}

/// The capture: a direct chat cleared with its media, starred messages not
/// kept apart, covering one message. The range is seconds on the wire.
#[test]
fn clear_chat_update_relays_the_index_flags_and_the_range_in_ms() {
    let mut key = MessageKey::default();
    key.remote_jid = Some(CHAT.to_string());
    key.from_me = Some(false);
    key.id = Some("00EA94CFC7CE262961CA68E1E42A33D1".to_string());
    let mut message = SyncActionMessage::default();
    message.key = key.into();
    message.timestamp = Some(1_791_458_172);
    let mut range = SyncActionMessageRange::default();
    range.last_message_timestamp = Some(1_791_458_172);
    range.messages = vec![message];
    let mut action = ClearChatAction::default();
    action.message_range = range.into();
    let event = Event::ClearChatUpdate(
        ClearChatUpdate::builder()
            .jid(lib(CHAT))
            .delete_starred(false)
            .delete_media(true)
            .timestamp(at_ms!(1_791_473_112_683))
            .action_timestamp(at_ms!(1_791_473_112_683))
            .action(Box::new(action))
            .from_full_sync(false)
            .build(),
    );
    let expected = header(
        CHAT,
        1_791_473_112_683,
        Action::ClearChat(pb::ClearChatChange {
            delete_starred: false,
            delete_media: true,
            message_range: Some(pb::SyncMessageRange {
                last_message_timestamp: Some(1_791_458_172_000),
                last_system_message_timestamp: None,
                messages: vec![pb::SyncRangeMessage {
                    key: Some(pb::MessageKey {
                        chat: wire(CHAT),
                        id: "00EA94CFC7CE262961CA68E1E42A33D1".to_string(),
                        from_me: false,
                        participant: None,
                    }),
                    timestamp: Some(1_791_458_172_000),
                }],
            }),
        }),
    );
    assert_eq!(app_state_of(event), expected);
}

fn delete_for_me_event(participant: Option<&str>, from_me: bool) -> Event {
    let mut action = DeleteMessageForMeAction::default();
    action.delete_media = Some(false);
    action.message_timestamp = Some(1_791_470_654);
    Event::DeleteMessageForMeUpdate(
        DeleteMessageForMeUpdate::builder()
            .chat_jid(lib(GROUP))
            .maybe_participant_jid(participant.map(lib))
            .message_id("A546B8B482B2ABE843F23ED7117CA390".to_string())
            .from_me(from_me)
            .timestamp(at_ms!(1_791_470_663_200))
            .action_timestamp(at_ms!(1_791_470_663_200))
            .action(Box::new(action))
            .from_full_sync(false)
            .build(),
    )
}

/// The capture: the user's own message in a group. The message time is
/// seconds on the wire (9 s before the mutation) and crosses in ms.
#[test]
fn delete_message_for_me_relays_the_key_and_the_message_time_in_ms() {
    let expected = header(
        GROUP,
        1_791_470_663_200,
        Action::DeleteMessageForMe(pb::DeleteMessageForMeChange {
            participant: None,
            message_id: "A546B8B482B2ABE843F23ED7117CA390".to_string(),
            from_me: true,
            delete_media: Some(false),
            message_timestamp: Some(1_791_470_654_000),
        }),
    );
    assert_eq!(app_state_of(delete_for_me_event(None, true)), expected);
}

#[test]
fn delete_message_for_me_relays_a_group_participant() {
    let update = app_state_of(delete_for_me_event(Some(SENDER), false));
    let Some(Action::DeleteMessageForMe(change)) = update.action else {
        panic!("expected a delete-for-me, got {:?}", update.action);
    };
    assert_eq!((change.participant, change.from_me), (wire(SENDER), false));
}

fn status_mute_event(muted: Option<bool>) -> Event {
    let mut action = UserStatusMuteAction::default();
    action.muted = muted;
    Event::UserStatusMuteUpdate(
        UserStatusMuteUpdate::builder()
            .jid(lib(CONTACT))
            .muted(muted.unwrap_or(false))
            .timestamp(at_ms!(1_791_470_702_203))
            .action_timestamp(at_ms!(1_791_470_702_203))
            .action(Box::new(action))
            .from_full_sync(false)
            .build(),
    )
}

/// The capture: hiding a contact from the status list sends `muted` true.
#[test]
fn user_status_mute_relays_the_action_flag() {
    let expected = header(
        CONTACT,
        1_791_470_702_203,
        Action::UserStatusMute(pb::UserStatusMuteChange { muted: Some(true) }),
    );
    assert_eq!(app_state_of(status_mute_event(Some(true))), expected);
}

/// The library reads an absent flag as false; the wire keeps it absent.
#[test]
fn user_status_mute_without_the_flag_leaves_muted_unset() {
    let update = app_state_of(status_mute_event(None));
    assert_eq!(
        update.action,
        Some(Action::UserStatusMute(pb::UserStatusMuteChange {
            muted: None
        }))
    );
}
