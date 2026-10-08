//! #134: the typed projection of each app-state struct, asserted as the whole
//! message. Values are the ones `pessoal` sent on 2026-10-07 18:42Z, when the
//! user archived, pinned, muted for 8 hours, starred and marked a chat unread
//! on the phone (read off the subscription; chat jids anonymized, message ids
//! as sent). The delete is hand-built: deleting a real chat was not done.

#![allow(clippy::field_reassign_with_default)] // the generated actions are built by assignment

use std::str::FromStr;

use wacore::types::events::{
    ArchiveUpdate, DeleteChatUpdate, MarkChatAsReadUpdate, MuteUpdate, PinUpdate, StarUpdate,
};
use whatsapp_rust::Jid;
use whatsapp_rust::waproto::whatsapp::MessageKey;
use whatsapp_rust::waproto::whatsapp::sync_action_value::{
    ArchiveChatAction, DeleteChatAction, MarkChatAsReadAction, MuteAction, PinAction, StarAction,
    SyncActionMessage, SyncActionMessageRange,
};

use super::*;
use crate::domain::event_mapping::map_event;
use crate::proto::v1::app_state_update::Action;
use crate::proto::v1::event_envelope::Event as PbEvent;

const GROUP: &str = "120363000000000134@g.us";
const SENDER: &str = "100000000000003@lid";

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

/// A range message as the capture had it: a key and its time in seconds.
fn range_message(id: &str, from_me: bool, sender: Option<&str>, seconds: i64) -> SyncActionMessage {
    let mut key = MessageKey::default();
    key.remote_jid = Some(GROUP.to_string());
    key.from_me = Some(from_me);
    key.id = Some(id.to_string());
    key.participant = sender.map(str::to_string);
    let mut message = SyncActionMessage::default();
    message.key = key.into();
    message.timestamp = Some(seconds);
    message
}

fn range_message_on_the_wire(
    id: &str,
    from_me: bool,
    sender: Option<&str>,
    ms: i64,
) -> pb::SyncRangeMessage {
    pb::SyncRangeMessage {
        key: Some(pb::MessageKey {
            chat: wire(GROUP),
            id: id.to_string(),
            from_me,
            participant: sender.and_then(wire),
        }),
        timestamp: Some(ms),
    }
}

/// The archive's range: the chat's last message, in seconds on the wire.
fn archive_range() -> SyncActionMessageRange {
    let mut range = SyncActionMessageRange::default();
    range.last_message_timestamp = Some(1_791_386_927);
    range.messages = vec![range_message(
        "AC233E41EEFC78B4285D2BC09DB26067",
        false,
        Some(SENDER),
        1_791_386_927,
    )];
    range
}

fn archive_range_on_the_wire() -> pb::SyncMessageRange {
    pb::SyncMessageRange {
        last_message_timestamp: Some(1_791_386_927_000),
        last_system_message_timestamp: None,
        messages: vec![range_message_on_the_wire(
            "AC233E41EEFC78B4285D2BC09DB26067",
            false,
            Some(SENDER),
            1_791_386_927_000,
        )],
    }
}

/// The header every update shares, with the action's time and its case.
fn header(ms: i64, action: Action) -> pb::AppStateUpdate {
    pb::AppStateUpdate {
        chat: wire(GROUP),
        timestamp: ms,
        action_timestamp: Some(ms),
        from_full_sync: false,
        action: Some(action),
    }
}

#[test]
fn archive_maps_the_captured_value() {
    let mut action = ArchiveChatAction::default();
    action.archived = Some(true);
    action.message_range = archive_range().into();
    let update = ArchiveUpdate::builder()
        .jid(lib(GROUP))
        .timestamp(at_ms!(1_791_398_525_501))
        .action_timestamp(at_ms!(1_791_398_525_501))
        .action(Box::new(action))
        .from_full_sync(false)
        .build();
    let expected = header(
        1_791_398_525_501,
        Action::Archive(pb::ArchiveChange {
            archived: Some(true),
            message_range: Some(archive_range_on_the_wire()),
        }),
    );
    assert_eq!(archive_update_of(&update), expected);
}

#[test]
fn pin_maps_the_captured_value() {
    let mut action = PinAction::default();
    action.pinned = Some(true);
    let update = PinUpdate::builder()
        .jid(lib(GROUP))
        .timestamp(at_ms!(1_791_398_533_080))
        .action_timestamp(at_ms!(1_791_398_533_080))
        .action(Box::new(action))
        .from_full_sync(false)
        .build();
    let expected = header(
        1_791_398_533_080,
        Action::Pin(pb::PinChange { pinned: Some(true) }),
    );
    assert_eq!(pin_update_of(&update), expected);
}

/// The end of an 8 h mute is already ms on the wire (measured: the mute time
/// plus 28 800 000), so it crosses as sent.
#[test]
fn mute_maps_the_end_in_ms_as_sent() {
    let mut action = MuteAction::default();
    action.muted = Some(true);
    action.mute_end_timestamp = Some(1_791_427_343_726);
    action.mute_everyone_mention_end_timestamp = Some(0);
    let update = MuteUpdate::builder()
        .jid(lib(GROUP))
        .timestamp(at_ms!(1_791_398_543_736))
        .action_timestamp(at_ms!(1_791_398_543_736))
        .action(Box::new(action))
        .from_full_sync(false)
        .build();
    let expected = header(
        1_791_398_543_736,
        Action::Mute(pb::MuteChange {
            muted: Some(true),
            mute_end_timestamp: Some(1_791_427_343_726),
            auto_muted: None,
            mute_everyone_mention_end_timestamp: Some(0),
        }),
    );
    assert_eq!(mute_update_of(&update), expected);
}

#[test]
fn unmute_leaves_the_end_unset() {
    let mut action = MuteAction::default();
    action.muted = Some(false);
    action.mute_everyone_mention_end_timestamp = Some(0);
    let update = MuteUpdate::builder()
        .jid(lib(GROUP))
        .timestamp(at_ms!(1_791_398_548_169))
        .action_timestamp(at_ms!(1_791_398_548_169))
        .action(Box::new(action))
        .from_full_sync(false)
        .build();
    let Some(Action::Mute(mute)) = mute_update_of(&update).action else {
        panic!("a mute");
    };
    assert_eq!(mute.muted, Some(false));
    assert_eq!(mute.mute_end_timestamp, None);
}

#[test]
fn star_maps_the_captured_value() {
    let mut action = StarAction::default();
    action.starred = Some(true);
    let update = StarUpdate::builder()
        .chat_jid(lib(GROUP))
        .message_id("A5499D5D0C98BDD2C40CA5D024DF94F0".to_string())
        .from_me(true)
        .timestamp(at_ms!(1_791_398_556_543))
        .action_timestamp(at_ms!(1_791_398_556_543))
        .action(Box::new(action))
        .from_full_sync(false)
        .build();
    let expected = header(
        1_791_398_556_543,
        Action::Star(pb::StarChange {
            participant: None,
            message_id: "A5499D5D0C98BDD2C40CA5D024DF94F0".to_string(),
            from_me: true,
            starred: Some(true),
        }),
    );
    assert_eq!(star_update_of(&update), expected);
}

/// Someone else's message in a group names its sender.
#[test]
fn star_of_someone_elses_message_names_the_sender() {
    let mut action = StarAction::default();
    action.starred = Some(false);
    let update = StarUpdate::builder()
        .chat_jid(lib(GROUP))
        .participant_jid(lib(SENDER))
        .message_id("3EB0B71AA9335B02599A9B".to_string())
        .from_me(false)
        .timestamp(at_ms!(1_791_398_556_543))
        .action(Box::new(action))
        .from_full_sync(true)
        .build();
    let mapped = star_update_of(&update);
    assert_eq!(mapped.chat, wire(GROUP));
    assert!(mapped.from_full_sync);
    let Some(Action::Star(star)) = mapped.action else {
        panic!("a star");
    };
    assert_eq!(star.participant, wire(SENDER));
    assert_eq!(star.starred, Some(false));
}

/// "Mark as unread" from the capture: no last timestamps, a list of messages.
#[test]
fn mark_read_maps_every_range_message() {
    let mut range = SyncActionMessageRange::default();
    range.messages = vec![
        range_message("3EB0DA537C42631B06611E", true, None, 1_790_171_762),
        range_message(
            "A55813927E37B661A7A43625615D333D",
            true,
            None,
            1_786_553_090,
        ),
    ];
    let mut action = MarkChatAsReadAction::default();
    action.read = Some(false);
    action.message_range = range.into();
    let update = MarkChatAsReadUpdate::builder()
        .jid(lib(GROUP))
        .timestamp(at_ms!(1_791_398_565_948))
        .action_timestamp(at_ms!(1_791_398_565_948))
        .action(Box::new(action))
        .from_full_sync(false)
        .build();
    let expected = header(
        1_791_398_565_948,
        Action::MarkRead(pb::MarkReadChange {
            read: Some(false),
            message_range: Some(pb::SyncMessageRange {
                last_message_timestamp: None,
                last_system_message_timestamp: None,
                messages: vec![
                    range_message_on_the_wire(
                        "3EB0DA537C42631B06611E",
                        true,
                        None,
                        1_790_171_762_000,
                    ),
                    range_message_on_the_wire(
                        "A55813927E37B661A7A43625615D333D",
                        true,
                        None,
                        1_786_553_090_000,
                    ),
                ],
            }),
        }),
    );
    assert_eq!(mark_read_update_of(&update), expected);
}

/// Hand-built: every field set, the system timestamp too.
#[test]
fn delete_chat_maps_delete_media_and_range() {
    let mut range = archive_range();
    range.last_system_message_timestamp = Some(1_791_386_900);
    let mut action = DeleteChatAction::default();
    action.message_range = range.into();
    let update = DeleteChatUpdate::builder()
        .jid(lib(GROUP))
        .delete_media(true)
        .timestamp(at_ms!(1_791_398_600_000))
        .action_timestamp(at_ms!(1_791_398_600_000))
        .action(Box::new(action))
        .from_full_sync(false)
        .build();
    let mut range_on_the_wire = archive_range_on_the_wire();
    range_on_the_wire.last_system_message_timestamp = Some(1_791_386_900_000);
    let expected = header(
        1_791_398_600_000,
        Action::DeleteChat(pb::DeleteChatChange {
            delete_media: true,
            message_range: Some(range_on_the_wire),
        }),
    );
    assert_eq!(delete_chat_update_of(&update), expected);
}

/// Upstream #1562: with no time in the mutation, `timestamp` is a fallback and
/// only an unset `action_timestamp` says so. An action without a range leaves
/// the range unset rather than an empty one.
#[test]
fn absent_fields_stay_unset() {
    let update = ArchiveUpdate::builder()
        .jid(lib(GROUP))
        .timestamp(at_ms!(0))
        .action(Box::default())
        .from_full_sync(false)
        .build();
    let expected = pb::AppStateUpdate {
        chat: wire(GROUP),
        timestamp: 0,
        action_timestamp: None,
        from_full_sync: false,
        action: Some(Action::Archive(pb::ArchiveChange {
            archived: None,
            message_range: None,
        })),
    };
    assert_eq!(archive_update_of(&update), expected);
}

/// The six library events reach the socket through these functions.
#[test]
fn every_app_state_event_maps_to_its_case() {
    use wacore::types::events::Event;
    let at = || at_ms!(1_791_398_525_501);
    let events = [
        Event::ArchiveUpdate(
            ArchiveUpdate::builder()
                .jid(lib(GROUP))
                .timestamp(at())
                .action(Box::default())
                .from_full_sync(false)
                .build(),
        ),
        Event::PinUpdate(
            PinUpdate::builder()
                .jid(lib(GROUP))
                .timestamp(at())
                .action(Box::default())
                .from_full_sync(false)
                .build(),
        ),
        Event::MuteUpdate(
            MuteUpdate::builder()
                .jid(lib(GROUP))
                .timestamp(at())
                .action(Box::default())
                .from_full_sync(false)
                .build(),
        ),
        Event::StarUpdate(
            StarUpdate::builder()
                .chat_jid(lib(GROUP))
                .message_id("M".to_string())
                .from_me(true)
                .timestamp(at())
                .action(Box::default())
                .from_full_sync(false)
                .build(),
        ),
        Event::MarkChatAsReadUpdate(
            MarkChatAsReadUpdate::builder()
                .jid(lib(GROUP))
                .timestamp(at())
                .action(Box::default())
                .from_full_sync(false)
                .build(),
        ),
        Event::DeleteChatUpdate(
            DeleteChatUpdate::builder()
                .jid(lib(GROUP))
                .delete_media(false)
                .timestamp(at())
                .action(Box::default())
                .from_full_sync(false)
                .build(),
        ),
    ];
    let cases: Vec<&str> = events
        .iter()
        .map(|event| match map_event(event).into_iter().next() {
            Some(PbEvent::AppState(pb::AppStateUpdate {
                action: Some(action),
                ..
            })) => match action {
                Action::Archive(_) => "archive",
                Action::Pin(_) => "pin",
                Action::Mute(_) => "mute",
                Action::Star(_) => "star",
                Action::MarkRead(_) => "mark_read",
                Action::DeleteChat(_) => "delete_chat",
                // #148: their own events, asserted in app_state_update_chat_tests.rs.
                Action::Lock(_) => "lock",
                Action::ClearChat(_) => "clear_chat",
                Action::DeleteMessageForMe(_) => "delete_message_for_me",
                Action::UserStatusMute(_) => "user_status_mute",
            },
            other => panic!("expected an app-state update, got {other:?}"),
        })
        .collect();
    assert_eq!(
        cases,
        ["archive", "pin", "mute", "star", "mark_read", "delete_chat"]
    );
}
