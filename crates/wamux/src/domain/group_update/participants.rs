//! The member-shaped payloads of a group notification (#133): the one list
//! shape seven actions share, and the entry it holds.

use wacore::stanza::groups::GroupParticipantInfo;
use wamux_types::group_enums::{history_sent_state_of, notification_participant_type_of};

use crate::domain::wire_time::millis_from_seconds;
use crate::proto::v1 as pb;
use wamux_types::{relay_lib_jid, relay_optional_lib_jid};

/// add, remove, promote, demote, modify and the two linked-group variants.
/// `reason` is only ever set by add and remove.
pub(super) fn participants_change(
    participants: &[GroupParticipantInfo],
    reason: Option<&String>,
) -> pb::GroupParticipantsChange {
    pb::GroupParticipantsChange {
        participants: notification_participants(participants),
        reason: reason.cloned(),
    }
}

pub(super) fn notification_participants(
    participants: &[GroupParticipantInfo],
) -> Vec<pb::GroupNotificationParticipant> {
    participants.iter().map(notification_participant).collect()
}

/// A missing type or history state is UNSPECIFIED, not a guessed default: a
/// join request's `<requested_user>` entries carry neither.
fn notification_participant(info: &GroupParticipantInfo) -> pb::GroupNotificationParticipant {
    pb::GroupNotificationParticipant {
        jid: relay_lib_jid(&info.jid),
        phone_number: relay_optional_lib_jid(info.phone_number.as_ref()),
        display_name: info.display_name.clone(),
        r#type: info
            .r#type
            .as_ref()
            .map_or(pb::GroupParticipantType::Unspecified, |kind| {
                notification_participant_type_of(kind)
            }) as i32,
        lid: relay_optional_lib_jid(info.lid.as_ref()),
        username: info.username.clone(),
        join_time: info.join_time.map(millis_from_seconds),
        group_history_sent_state: info.group_history_sent_state.map_or(
            pb::GroupHistorySentState::Unspecified,
            history_sent_state_of,
        ) as i32,
    }
}
