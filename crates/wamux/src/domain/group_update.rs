//! A group notification's action, projected onto the contract's typed
//! `GroupUpdate` (#133, part 2 of #74). It used to cross as the library's
//! `serde` JSON in `raw`, which spelled every jid as the library's struct.

mod participants;

use wacore::iq::groups::GroupMetadataResponse;
use wacore::protocol::ProtocolNode;
use wacore::stanza::groups::GroupNotificationAction as Lib;
use wacore::types::events::GroupUpdate;
use wamux_types::group_enums::{member_add_mode_token_of, membership_request_method_of};
use whatsapp_rust::GroupMetadata;
use whatsapp_rust::wacore_binary::Node;

use self::participants::{notification_participants, participants_change};
use crate::domain::group_metadata::{group_metadata_of, lib_jid};
use crate::domain::wire_time::millis_from_seconds;
use crate::proto::v1 as pb;
use crate::proto::v1::group_update::Action;

/// The notification fields and the action, one oneof case per library variant.
pub fn group_update_of(update: &GroupUpdate) -> pb::GroupUpdate {
    pb::GroupUpdate {
        group: lib_jid(&update.group_jid),
        notification_id: update.notification_id.clone(),
        notify: update.notify.clone(),
        offline: update.offline.clone(),
        action_index: update.action_index,
        participant: update.participant.as_ref().and_then(lib_jid),
        participant_pn: update.participant_pn.as_ref().and_then(lib_jid),
        participant_username: update.participant_username.clone(),
        participant_country_code: update.participant_country_code.clone(),
        timestamp: update.timestamp.timestamp_millis(),
        is_lid_addressing_mode: update.is_lid_addressing_mode,
        has_incomplete_participant_information: update.has_incomplete_participant_information,
        action: Some(action_of(&update.action)),
    }
}

/// One arm per library variant and no wildcard: a variant added upstream
/// breaks the build at the bump instead of reaching the edge as nothing.
/// Long on purpose, each arm only delegates or names a case.
fn action_of(action: &Lib) -> Action {
    match action {
        Lib::Add {
            participants,
            reason,
        } => Action::Add(participants_change(participants, reason.as_ref())),
        Lib::Remove {
            participants,
            reason,
        } => Action::Remove(participants_change(participants, reason.as_ref())),
        Lib::Promote { participants } => Action::Promote(participants_change(participants, None)),
        Lib::Demote { participants } => Action::Demote(participants_change(participants, None)),
        Lib::Modify { participants } => Action::Modify(participants_change(participants, None)),
        Lib::Subject {
            subject,
            subject_owner,
            subject_owner_pn,
            subject_owner_username,
            subject_time,
        } => Action::Subject(pb::GroupSubjectChange {
            subject: subject.clone(),
            subject_owner: subject_owner.as_ref().and_then(lib_jid),
            subject_owner_pn: subject_owner_pn.as_ref().and_then(lib_jid),
            subject_owner_username: subject_owner_username.clone(),
            subject_time: subject_time.map(millis_from_seconds),
        }),
        Lib::Description { id, description } => Action::Description(pb::GroupDescriptionChange {
            id: id.clone(),
            description: description.clone(),
        }),
        Lib::Locked { threshold } => Action::Locked(pb::GroupLockedChange {
            threshold: threshold.clone(),
        }),
        Lib::Unlocked => Action::Unlocked(pb::Empty {}),
        Lib::Announce => Action::Announcement(pb::Empty {}),
        Lib::NotAnnounce => Action::NotAnnouncement(pb::Empty {}),
        Lib::Ephemeral {
            expiration,
            trigger,
        } => Action::Ephemeral(pb::GroupEphemeralSettings {
            expiration_seconds: Some(*expiration),
            trigger: *trigger,
        }),
        Lib::MembershipApprovalMode { enabled } => {
            Action::MembershipApprovalMode(pb::GroupMembershipApprovalMode { enabled: *enabled })
        }
        Lib::MembershipApprovalRequest {
            request_method,
            parent_group_jid,
        } => Action::MembershipApprovalRequest(pb::GroupMembershipApprovalRequest {
            request_method: membership_request_method_of(request_method) as i32,
            parent_group: parent_group_jid.as_ref().and_then(lib_jid),
        }),
        Lib::CreatedMembershipRequests {
            request_method,
            parent_group_jid,
            requests,
        } => Action::CreatedMembershipRequests(pb::GroupCreatedMembershipRequests {
            request_method: membership_request_method_of(request_method) as i32,
            parent_group: parent_group_jid.as_ref().and_then(lib_jid),
            requests: notification_participants(requests),
        }),
        Lib::RevokedMembershipRequests { participants } => {
            Action::RevokedMembershipRequests(pb::GroupRevokedMembershipRequests {
                participants: participants.iter().filter_map(lib_jid).collect(),
            })
        }
        Lib::MemberAddMode { mode } => Action::MemberAddMode(member_add_mode_change(mode)),
        Lib::NoFrequentlyForwarded => Action::NoFrequentlyForwarded(pb::Empty {}),
        Lib::FrequentlyForwardedOk => Action::FrequentlyForwardedOk(pb::Empty {}),
        Lib::Invite { code } => Action::Invite(pb::GroupInviteJoin { code: code.clone() }),
        Lib::RevokeInvite => Action::Revoke(pb::Empty {}),
        // The unit of `expiration` is not verified, so it crosses verbatim.
        Lib::GrowthLocked {
            expiration,
            lock_type,
        } => Action::GrowthLocked(pb::GrowthLockInfo {
            lock_type: lock_type.clone(),
            expiration: u64::from(*expiration),
        }),
        Lib::GrowthUnlocked => Action::GrowthUnlocked(pb::Empty {}),
        Lib::Create { raw } => Action::Create(group_created_of(raw)),
        Lib::Delete { reason } => Action::Delete(pb::GroupDeleted {
            reason: reason.clone(),
        }),
        // The library keeps the rest of the node unparsed (`raw` is
        // `#[wire(skip)]`), so only the typed fields cross.
        Lib::Link { link_type, .. } => Action::Link(pb::GroupLinked {
            link_type: link_type.clone(),
        }),
        Lib::Unlink {
            unlink_type,
            unlink_reason,
            ..
        } => Action::Unlink(pb::GroupUnlinked {
            unlink_type: unlink_type.clone(),
            unlink_reason: unlink_reason.clone(),
        }),
        Lib::LinkedGroupPromote { participants } => {
            Action::LinkedGroupPromote(participants_change(participants, None))
        }
        Lib::LinkedGroupDemote { participants } => {
            Action::LinkedGroupDemote(participants_change(participants, None))
        }
        Lib::Suspended => Action::Suspended(pb::Empty {}),
        Lib::Unsuspended => Action::Unsuspended(pb::Empty {}),
        Lib::AutoAddDisabled => Action::AutoAddDisabled(pb::Empty {}),
        Lib::IsCapiHostedGroup => Action::IsCapiHostedGroup(pb::Empty {}),
        Lib::GroupSafetyCheck => Action::GroupSafetyCheck(pb::Empty {}),
        Lib::LimitSharingEnabled { trigger } => {
            Action::LimitSharingEnabled(pb::GroupLimitSharing { trigger: *trigger })
        }
        Lib::AllowAdminReports => Action::AllowAdminReports(pb::Empty {}),
        Lib::NotAllowAdminReports => Action::NotAllowAdminReports(pb::Empty {}),
        Lib::Reports => Action::Reports(pb::Empty {}),
        Lib::AllowNonAdminSubGroupCreation => Action::AllowNonAdminSubGroupCreation(pb::Empty {}),
        Lib::NotAllowNonAdminSubGroupCreation => {
            Action::NotAllowNonAdminSubGroupCreation(pb::Empty {})
        }
        Lib::CreatedSubGroupSuggestion { .. } => Action::CreatedSubGroupSuggestion(pb::Empty {}),
        Lib::RevokedSubGroupSuggestions { .. } => Action::RevokedSubGroupSuggestions(pb::Empty {}),
        Lib::ChangeNumber {
            new_owner,
            sub_group_suggestions,
        } => Action::ChangeNumber(pb::GroupChangeNumber {
            new_owner: new_owner.as_ref().and_then(lib_jid),
            sub_group_suggestions: sub_group_suggestions.iter().filter_map(lib_jid).collect(),
        }),
        Lib::Unknown { tag } => Action::Unknown(pb::GroupUnknownAction { tag: tag.clone() }),
    }
}

fn member_add_mode_change(token: &str) -> pb::GroupMemberAddModeChange {
    let (mode, mode_raw) = member_add_mode_token_of(token);
    pb::GroupMemberAddModeChange {
        mode: mode as i32,
        mode_raw,
    }
}

/// The `<group>` child goes through the library's public parser, the one
/// GetGroupMetadata uses. No `<group>`, or one the parser rejects, is relayed
/// as the fact (unset metadata), never as an invented empty group.
fn group_created_of(create: &Node) -> pb::GroupCreated {
    let metadata = create
        .get_optional_child("group")
        .and_then(|group| GroupMetadataResponse::try_from_node(group).ok())
        .map(GroupMetadata::from)
        .map(|metadata| group_metadata_of(&metadata));
    pb::GroupCreated { metadata }
}

#[cfg(test)]
#[path = "group_update_tests.rs"]
mod tests;
