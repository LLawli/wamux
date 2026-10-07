//! A group as the library hands it over, projected onto the contract's typed
//! `GroupMetadata` (#132, part 1 of #74). It used to cross as hand-built JSON
//! with five fields, and hand-picking is exactly what once dropped each
//! participant's identity (issue #1): a LID-addressed group hands back every
//! member as a `@lid` with `phone_number` alongside, and flattening a
//! participant to its jid string threw away the answer to "who is this @lid"
//! for the whole roster, plus who is admin. So every field crosses, and an
//! absent one stays unset rather than guessed.

use wacore::iq::groups::{GroupEphemeralSettings, GroupParticipantDetails, GrowthLockInfo};
use wamux_types::group_enums::{
    addressing_mode_of, appeal_status_of, member_add_mode_of, member_link_mode_of,
    member_share_history_mode_of, participant_type_of,
};
use wamux_types::relay_jid;
use whatsapp_rust::features::{GroupParticipant, MembershipRequest};
use whatsapp_rust::{GroupMetadata, Jid};

use crate::domain::wire_time::millis_from_seconds;
use crate::proto::v1 as pb;

/// Every field of the library's metadata, participants whole. Split by
/// concern and joined with struct update; each helper sets only its own
/// fields, so a field no helper owns would surface as a proto default in the
/// tests, which compare the whole message.
pub fn group_metadata_of(metadata: &GroupMetadata) -> pb::GroupMetadata {
    pb::GroupMetadata {
        id: lib_jid(&metadata.id),
        subject: metadata.subject.clone(),
        notify: metadata.notify.clone(),
        participants: metadata.participants.iter().map(participant_of).collect(),
        addressing_mode: addressing_mode_of(metadata.addressing_mode) as i32,
        ..creator_part(metadata)
    }
}

fn creator_part(md: &GroupMetadata) -> pb::GroupMetadata {
    pb::GroupMetadata {
        creator: optional_lib_jid(md.creator.as_ref()),
        creator_pn: optional_lib_jid(md.creator_pn.as_ref()),
        creator_username: md.creator_username.clone(),
        creator_country_code: md.creator_country_code.clone(),
        creation_time: md.creation_time.map(millis_from_seconds),
        participant_version_id: md.participant_version_id.clone(),
        admin_version_id: md.admin_version_id.clone(),
        open_thread_id: md.open_thread_id.clone(),
        has_missing_participant_identification: md.has_missing_participant_identification,
        ..subject_part(md)
    }
}

fn subject_part(md: &GroupMetadata) -> pb::GroupMetadata {
    pb::GroupMetadata {
        subject_time: md.subject_time.map(millis_from_seconds),
        subject_owner: optional_lib_jid(md.subject_owner.as_ref()),
        subject_owner_pn: optional_lib_jid(md.subject_owner_pn.as_ref()),
        subject_owner_username: md.subject_owner_username.clone(),
        description: md.description.clone(),
        description_id: md.description_id.clone(),
        description_owner: optional_lib_jid(md.description_owner.as_ref()),
        description_owner_pn: optional_lib_jid(md.description_owner_pn.as_ref()),
        description_owner_username: md.description_owner_username.clone(),
        description_time: md.description_time.map(millis_from_seconds),
        ..settings_part(md)
    }
}

fn settings_part(md: &GroupMetadata) -> pb::GroupMetadata {
    pb::GroupMetadata {
        is_locked: md.is_locked,
        is_announcement: md.is_announcement,
        ephemeral: md.ephemeral.as_ref().map(ephemeral_of),
        membership_approval: md.membership_approval,
        member_add_mode: md
            .member_add_mode
            .map_or(pb::GroupMemberAddMode::Unspecified, member_add_mode_of)
            as i32,
        member_link_mode: md
            .member_link_mode
            .map_or(pb::GroupMemberLinkMode::Unspecified, member_link_mode_of)
            as i32,
        size: md.size,
        no_frequently_forwarded: md.no_frequently_forwarded,
        member_share_history_mode: md.member_share_history_mode.map_or(
            pb::GroupMemberShareHistoryMode::Unspecified,
            member_share_history_mode_of,
        ) as i32,
        ..community_part(md)
    }
}

fn community_part(md: &GroupMetadata) -> pb::GroupMetadata {
    pb::GroupMetadata {
        is_parent_group: md.is_parent_group,
        parent_membership_approval_required: md.parent_membership_approval_required,
        parent_group: optional_lib_jid(md.parent_group_jid.as_ref()),
        is_default_sub_group: md.is_default_sub_group,
        is_general_chat: md.is_general_chat,
        allow_non_admin_sub_group_creation: md.allow_non_admin_sub_group_creation,
        ..moderation_part(md)
    }
}

fn moderation_part(md: &GroupMetadata) -> pb::GroupMetadata {
    pb::GroupMetadata {
        growth_locked: md.growth_locked.as_ref().map(growth_lock_of),
        is_suspended: md.is_suspended,
        suspension_can_auto_file: md.suspension_can_auto_file,
        appeal_status: md
            .appeal_status
            .map_or(pb::GroupAppealStatus::Unspecified, appeal_status_of)
            as i32,
        // Unit not verified (#132): relayed as the server sent it.
        appeal_update_time: md.appeal_update_time,
        is_support_group: md.is_support_group,
        allow_admin_reports: md.allow_admin_reports,
        ..flags_part(md)
    }
}

fn flags_part(md: &GroupMetadata) -> pb::GroupMetadata {
    pb::GroupMetadata {
        is_hidden_group: md.is_hidden_group,
        is_incognito: md.is_incognito,
        has_group_history: md.has_group_history,
        is_auto_add_disabled: md.is_auto_add_disabled,
        has_capi: md.has_capi,
        evolution_version: md.evolution_version,
        has_group_safety_check: md.has_group_safety_check,
        participant_label_enabled: md.participant_label_enabled,
        is_limit_sharing_enabled: md.is_limit_sharing_enabled,
        limit_sharing_trigger: md.limit_sharing_trigger,
        ..pb::GroupMetadata::default()
    }
}

fn participant_of(participant: &GroupParticipant) -> pb::GroupParticipant {
    pb::GroupParticipant {
        jid: lib_jid(&participant.jid),
        phone_number: optional_lib_jid(participant.phone_number.as_ref()),
        lid: optional_lib_jid(participant.lid.as_ref()),
        username: participant.username.as_ref().map(ToString::to_string),
        r#type: participant_type_of(participant.participant_type) as i32,
        details: participant.details.as_deref().map(details_of),
    }
}

fn details_of(details: &GroupParticipantDetails) -> pb::GroupParticipantDetails {
    pb::GroupParticipantDetails {
        participant_label: details.participant_label.as_ref().map(ToString::to_string),
        // Unit not verified (#132): relayed as the server sent it.
        participant_label_mtime: details.participant_label_mtime,
        join_time: details.join_time.map(millis_from_seconds),
        group_history_sent: details.group_history_sent,
        display_name: details.display_name.as_ref().map(ToString::to_string),
        is_addressable: details.is_addressable,
    }
}

/// `expiration` is a duration in seconds, so it is renamed and not converted.
fn ephemeral_of(settings: &GroupEphemeralSettings) -> pb::GroupEphemeralSettings {
    pb::GroupEphemeralSettings {
        expiration_seconds: settings.expiration,
        trigger: settings.trigger,
    }
}

/// Unit and meaning of `expiration` are not verified (#132): verbatim.
fn growth_lock_of(lock: &GrowthLockInfo) -> pb::GrowthLockInfo {
    pb::GrowthLockInfo {
        lock_type: lock.lock_type.clone(),
        expiration: lock.expiration,
    }
}

/// One pending request: the jid as the contract's `Jid` (it used to cross as
/// the library's struct, #96) and the request time in milliseconds.
pub fn membership_request_of(request: &MembershipRequest) -> pb::MembershipRequest {
    pb::MembershipRequest {
        jid: lib_jid(&request.jid),
        request_time: request.request_time.map(millis_from_seconds),
    }
}

/// A lib jid as the wire message, verbatim (#122).
fn lib_jid(jid: &Jid) -> Option<pb::Jid> {
    relay_jid(jid.to_string())
}

/// An absent jid is an unset field, never an empty value (#122).
fn optional_lib_jid(jid: Option<&Jid>) -> Option<pb::Jid> {
    jid.and_then(lib_jid)
}

#[cfg(test)]
#[path = "group_metadata_tests.rs"]
mod tests;
