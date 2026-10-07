//! App-state chat mutations, projected onto the contract's typed
//! `AppStateUpdate` (#134, part 3 of #74). They used to cross as the library's
//! JSON in `raw`, built by one generic helper for all six; each library
//! struct now has its own concrete function.

use wacore::types::events::{
    ArchiveUpdate, DeleteChatUpdate, MarkChatAsReadUpdate, MuteUpdate, PinUpdate, StarUpdate,
};
use whatsapp_rust::Jid;
use whatsapp_rust::waproto::whatsapp::sync_action_value::{
    SyncActionMessage, SyncActionMessageRange,
};

use crate::domain::event_mapping::wa_key_to_proto;
use crate::domain::group_metadata::lib_jid;
use crate::domain::wire_time::millis_from_signed_seconds;

use crate::proto::v1 as pb;
use pb::app_state_update::Action;

/// The fields every update shares. The timestamps arrive as the library's
/// date type, which wamux does not depend on and so cannot name: the caller
/// reads them as ms (#134).
fn app_state_of(
    chat: &Jid,
    timestamp_ms: i64,
    action_timestamp_ms: Option<i64>,
    from_full_sync: bool,
    action: Action,
) -> pb::AppStateUpdate {
    pb::AppStateUpdate {
        chat: lib_jid(chat),
        timestamp: timestamp_ms,
        action_timestamp: action_timestamp_ms,
        from_full_sync,
        action: Some(action),
    }
}

/// The range's times are seconds on the wire (measured live 2026-10-07, #134)
/// and `last_system_message_timestamp` is read like its sibling (inferred, it
/// was never set). An unset range stays unset, not an empty one.
fn message_range_of(range: Option<&SyncActionMessageRange>) -> Option<pb::SyncMessageRange> {
    let range = range?;
    Some(pb::SyncMessageRange {
        last_message_timestamp: range.last_message_timestamp.map(millis_from_signed_seconds),
        last_system_message_timestamp: range
            .last_system_message_timestamp
            .map(millis_from_signed_seconds),
        messages: range.messages.iter().map(range_message_of).collect(),
    })
}

fn range_message_of(message: &SyncActionMessage) -> pb::SyncRangeMessage {
    pb::SyncRangeMessage {
        key: message.key.as_option().map(wa_key_to_proto),
        timestamp: message.timestamp.map(millis_from_signed_seconds),
    }
}

pub fn archive_update_of(update: &ArchiveUpdate) -> pb::AppStateUpdate {
    let action = Action::Archive(pb::ArchiveChange {
        archived: update.action.archived,
        message_range: message_range_of(update.action.message_range.as_option()),
    });
    app_state_of(
        &update.jid,
        update.timestamp.timestamp_millis(),
        update.action_timestamp.map(|at| at.timestamp_millis()),
        update.from_full_sync,
        action,
    )
}

pub fn pin_update_of(update: &PinUpdate) -> pb::AppStateUpdate {
    let action = Action::Pin(pb::PinChange {
        pinned: update.action.pinned,
    });
    app_state_of(
        &update.jid,
        update.timestamp.timestamp_millis(),
        update.action_timestamp.map(|at| at.timestamp_millis()),
        update.from_full_sync,
        action,
    )
}

/// `mute_end_timestamp` is already ms on the wire (measured live 2026-10-07:
/// an 8 h mute ended at the mute time + 28 800 000), so it crosses as sent;
/// the mention one has no verified unit and crosses verbatim.
pub fn mute_update_of(update: &MuteUpdate) -> pb::AppStateUpdate {
    let action = Action::Mute(pb::MuteChange {
        muted: update.action.muted,
        mute_end_timestamp: update.action.mute_end_timestamp,
        auto_muted: update.action.auto_muted,
        mute_everyone_mention_end_timestamp: update.action.mute_everyone_mention_end_timestamp,
    });
    app_state_of(
        &update.jid,
        update.timestamp.timestamp_millis(),
        update.action_timestamp.map(|at| at.timestamp_millis()),
        update.from_full_sync,
        action,
    )
}

pub fn star_update_of(update: &StarUpdate) -> pb::AppStateUpdate {
    let action = Action::Star(pb::StarChange {
        participant: update.participant_jid.as_ref().and_then(lib_jid),
        message_id: update.message_id.clone(),
        from_me: update.from_me,
        starred: update.action.starred,
    });
    app_state_of(
        &update.chat_jid,
        update.timestamp.timestamp_millis(),
        update.action_timestamp.map(|at| at.timestamp_millis()),
        update.from_full_sync,
        action,
    )
}

pub fn mark_read_update_of(update: &MarkChatAsReadUpdate) -> pb::AppStateUpdate {
    let action = Action::MarkRead(pb::MarkReadChange {
        read: update.action.read,
        message_range: message_range_of(update.action.message_range.as_option()),
    });
    app_state_of(
        &update.jid,
        update.timestamp.timestamp_millis(),
        update.action_timestamp.map(|at| at.timestamp_millis()),
        update.from_full_sync,
        action,
    )
}

pub fn delete_chat_update_of(update: &DeleteChatUpdate) -> pb::AppStateUpdate {
    let action = Action::DeleteChat(pb::DeleteChatChange {
        delete_media: update.delete_media,
        message_range: message_range_of(update.action.message_range.as_option()),
    });
    app_state_of(
        &update.jid,
        update.timestamp.timestamp_millis(),
        update.action_timestamp.map(|at| at.timestamp_millis()),
        update.from_full_sync,
        action,
    )
}

#[cfg(test)]
#[path = "app_state_update_tests.rs"]
mod tests;
