//! #132: the typed projection of a library `GroupMetadata`, asserted as the
//! whole message so a field the mapping forgets shows up as a difference.
//! The library value sets every field to something other than its default;
//! the literal below names every field (no `..Default::default()`), so a
//! field added upstream or to the proto breaks this test at compile time.

use std::str::FromStr;

use wacore::iq::groups::{
    AddressingMode, GroupAppealStatus, GroupEphemeralSettings, GroupParticipantDetails,
    GrowthLockInfo, MemberAddMode, MemberLinkMode, MemberShareHistoryMode, ParticipantType,
};
use whatsapp_rust::features::{GroupParticipant, MembershipRequest};
use whatsapp_rust::{GroupMetadata, Jid};

use super::*;

const GROUP: &str = "120363000000000068@g.us";
const PARENT: &str = "120363000000000069@g.us";
const MEMBER_LID: &str = "100000000000001@lid";
const MEMBER_PN: &str = "5511900000001@s.whatsapp.net";
const OWNER_LID: &str = "100000000000002@lid";
const OWNER_PN: &str = "5511900000002@s.whatsapp.net";

fn lib(value: &str) -> Jid {
    Jid::from_str(value).expect("test jid parses")
}

/// A jid as the core relays it out (#121): the value verbatim.
fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

/// Every detail away from its default (`is_addressable` defaults to true).
/// The struct is `#[non_exhaustive]`, so it can only be built by assignment.
#[allow(clippy::field_reassign_with_default)]
fn every_detail() -> GroupParticipantDetails {
    let mut details = GroupParticipantDetails::default();
    details.participant_label = Some("label".into());
    details.participant_label_mtime = Some(1_790_947_603);
    details.join_time = Some(1_790_947_599);
    details.group_history_sent = Some(false);
    details.display_name = Some("+55∙∙∙∙∙∙∙∙∙01".into());
    details.is_addressable = false;
    details
}

fn every_detail_on_the_wire() -> pb::GroupParticipantDetails {
    pb::GroupParticipantDetails {
        participant_label: Some("label".to_string()),
        // Unit not verified: verbatim.
        participant_label_mtime: Some(1_790_947_603),
        join_time: Some(1_790_947_599_000),
        group_history_sent: Some(false),
        display_name: Some("+55∙∙∙∙∙∙∙∙∙01".to_string()),
        is_addressable: false,
    }
}

/// REGRESSION (issue #1): a LID-addressed member keeps the phone number the
/// roster carries beside the `@lid`, and the admin role with it.
fn member_with_details() -> GroupParticipant {
    GroupParticipant {
        jid: lib(MEMBER_LID),
        phone_number: Some(lib(MEMBER_PN)),
        lid: Some(lib(MEMBER_LID)),
        username: Some("fulano".into()),
        participant_type: ParticipantType::SuperAdmin,
        details: Some(Box::new(every_detail())),
    }
}

fn member_with_details_on_the_wire() -> pb::GroupParticipant {
    pb::GroupParticipant {
        jid: wire(MEMBER_LID),
        phone_number: wire(MEMBER_PN),
        lid: wire(MEMBER_LID),
        username: Some("fulano".to_string()),
        r#type: pb::GroupParticipantType::Superadmin as i32,
        details: Some(every_detail_on_the_wire()),
    }
}

fn every_field() -> GroupMetadata {
    GroupMetadata {
        id: lib(GROUP),
        subject: Some("subject".to_string()),
        notify: Some("notify".to_string()),
        participants: vec![member_with_details()],
        addressing_mode: AddressingMode::Lid,
        creator: Some(lib(OWNER_LID)),
        creator_pn: Some(lib(OWNER_PN)),
        creator_username: Some("creator.user".to_string()),
        creator_country_code: Some("BR".to_string()),
        creation_time: Some(1_790_947_599),
        participant_version_id: Some("1790947897308104".to_string()),
        admin_version_id: Some("1790947599193645".to_string()),
        open_thread_id: Some("thread".to_string()),
        has_missing_participant_identification: true,
        subject_time: Some(1_790_947_600),
        subject_owner: Some(lib(MEMBER_LID)),
        subject_owner_pn: Some(lib(MEMBER_PN)),
        subject_owner_username: Some("subject.user".to_string()),
        description: Some("description".to_string()),
        description_id: Some("DESC1".to_string()),
        description_owner: Some(lib(OWNER_LID)),
        description_owner_pn: Some(lib(OWNER_PN)),
        description_owner_username: Some("description.user".to_string()),
        description_time: Some(1_790_947_601),
        is_locked: true,
        is_announcement: true,
        ephemeral: Some(GroupEphemeralSettings {
            expiration: Some(86_400),
            trigger: Some(2),
        }),
        membership_approval: true,
        member_add_mode: Some(MemberAddMode::AllMemberAdd),
        member_link_mode: Some(MemberLinkMode::AdminLink),
        size: Some(3),
        is_parent_group: true,
        parent_membership_approval_required: true,
        parent_group_jid: Some(lib(PARENT)),
        is_default_sub_group: true,
        is_general_chat: true,
        allow_non_admin_sub_group_creation: true,
        no_frequently_forwarded: true,
        member_share_history_mode: Some(MemberShareHistoryMode::AllMemberShare),
        growth_locked: Some(GrowthLockInfo {
            lock_type: "invite".to_string(),
            expiration: 1_790_950_000,
        }),
        is_suspended: true,
        suspension_can_auto_file: true,
        appeal_status: Some(GroupAppealStatus::InReview),
        appeal_update_time: Some(1_790_947_602),
        is_support_group: true,
        allow_admin_reports: true,
        is_hidden_group: true,
        is_incognito: true,
        has_group_history: true,
        is_auto_add_disabled: true,
        has_capi: true,
        evolution_version: Some(4),
        has_group_safety_check: true,
        participant_label_enabled: true,
        is_limit_sharing_enabled: true,
        limit_sharing_trigger: Some(7),
    }
}

fn every_field_on_the_wire() -> pb::GroupMetadata {
    pb::GroupMetadata {
        id: wire(GROUP),
        subject: Some("subject".to_string()),
        notify: Some("notify".to_string()),
        participants: vec![member_with_details_on_the_wire()],
        addressing_mode: pb::GroupAddressingMode::Lid as i32,
        creator: wire(OWNER_LID),
        creator_pn: wire(OWNER_PN),
        creator_username: Some("creator.user".to_string()),
        creator_country_code: Some("BR".to_string()),
        creation_time: Some(1_790_947_599_000),
        participant_version_id: Some("1790947897308104".to_string()),
        admin_version_id: Some("1790947599193645".to_string()),
        open_thread_id: Some("thread".to_string()),
        has_missing_participant_identification: true,
        subject_time: Some(1_790_947_600_000),
        subject_owner: wire(MEMBER_LID),
        subject_owner_pn: wire(MEMBER_PN),
        subject_owner_username: Some("subject.user".to_string()),
        description: Some("description".to_string()),
        description_id: Some("DESC1".to_string()),
        description_owner: wire(OWNER_LID),
        description_owner_pn: wire(OWNER_PN),
        description_owner_username: Some("description.user".to_string()),
        description_time: Some(1_790_947_601_000),
        is_locked: true,
        is_announcement: true,
        ephemeral: Some(pb::GroupEphemeralSettings {
            expiration_seconds: Some(86_400),
            trigger: Some(2),
        }),
        membership_approval: true,
        member_add_mode: pb::GroupMemberAddMode::AllMemberAdd as i32,
        member_link_mode: pb::GroupMemberLinkMode::AdminLink as i32,
        size: Some(3),
        is_parent_group: true,
        parent_membership_approval_required: true,
        parent_group: wire(PARENT),
        is_default_sub_group: true,
        is_general_chat: true,
        allow_non_admin_sub_group_creation: true,
        no_frequently_forwarded: true,
        member_share_history_mode: pb::GroupMemberShareHistoryMode::AllMemberShare as i32,
        // Unit not verified: verbatim.
        growth_locked: Some(pb::GrowthLockInfo {
            lock_type: "invite".to_string(),
            expiration: 1_790_950_000,
        }),
        is_suspended: true,
        suspension_can_auto_file: true,
        appeal_status: pb::GroupAppealStatus::InReview as i32,
        // Unit not verified: verbatim.
        appeal_update_time: Some(1_790_947_602),
        is_support_group: true,
        allow_admin_reports: true,
        is_hidden_group: true,
        is_incognito: true,
        has_group_history: true,
        is_auto_add_disabled: true,
        has_capi: true,
        evolution_version: Some(4),
        has_group_safety_check: true,
        participant_label_enabled: true,
        is_limit_sharing_enabled: true,
        limit_sharing_trigger: Some(7),
    }
}

#[test]
fn group_metadata_maps_every_field() {
    assert_eq!(group_metadata_of(&every_field()), every_field_on_the_wire());
}

/// Nothing the library left out is invented: every optional stays unset and
/// every optional enum UNSPECIFIED. `addressing_mode` is not optional in the
/// library, so its default (PN) crosses as PN.
#[test]
fn group_metadata_absent_fields_stay_unset() {
    let metadata = GroupMetadata {
        id: lib(GROUP),
        ..GroupMetadata::default()
    };
    let expected = pb::GroupMetadata {
        id: wire(GROUP),
        addressing_mode: pb::GroupAddressingMode::Pn as i32,
        ..pb::GroupMetadata::default()
    };
    assert_eq!(group_metadata_of(&metadata), expected);
}

#[test]
fn participant_maps_every_field_with_details() {
    let metadata = GroupMetadata {
        id: lib(GROUP),
        participants: vec![member_with_details()],
        ..GroupMetadata::default()
    };
    assert_eq!(
        group_metadata_of(&metadata).participants,
        vec![member_with_details_on_the_wire()]
    );
}

#[test]
fn participant_without_details_leaves_details_unset() {
    let bare = GroupParticipant {
        jid: lib(MEMBER_PN),
        phone_number: None,
        lid: None,
        username: None,
        participant_type: ParticipantType::Member,
        details: None,
    };
    let metadata = GroupMetadata {
        id: lib(GROUP),
        participants: vec![bare],
        ..GroupMetadata::default()
    };
    let expected = pb::GroupParticipant {
        jid: wire(MEMBER_PN),
        r#type: pb::GroupParticipantType::Member as i32,
        ..pb::GroupParticipant::default()
    };
    assert_eq!(group_metadata_of(&metadata).participants, vec![expected]);
}

/// The three values whose unit is not verified cross as the server sent them,
/// and the verified seconds next to them are converted.
#[test]
fn unverified_time_units_relay_verbatim() {
    let mapped = group_metadata_of(&every_field());
    assert_eq!(mapped.appeal_update_time, Some(1_790_947_602));
    assert_eq!(
        mapped.growth_locked.map(|g| g.expiration),
        Some(1_790_950_000)
    );
    let details = mapped.participants[0].details.clone();
    assert_eq!(
        details.and_then(|d| d.participant_label_mtime),
        Some(1_790_947_603)
    );
    assert_eq!(mapped.creation_time, Some(1_790_947_599_000));
}

#[test]
fn membership_request_maps_jid_and_request_time_in_ms() {
    let request = MembershipRequest {
        jid: lib(OWNER_LID),
        request_time: Some(1_790_947_901),
    };
    let expected = pb::MembershipRequest {
        jid: wire(OWNER_LID),
        request_time: Some(1_790_947_901_000),
    };
    assert_eq!(membership_request_of(&request), expected);
}

#[test]
fn membership_request_without_time_leaves_it_unset() {
    let request = MembershipRequest {
        jid: lib(OWNER_LID),
        request_time: None,
    };
    let expected = pb::MembershipRequest {
        jid: wire(OWNER_LID),
        request_time: None,
    };
    assert_eq!(membership_request_of(&request), expected);
}
