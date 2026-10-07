//! #132: every value the library names maps to its own enum value. The sets
//! are closed, so there is no UNKNOWN case to test.

use wacore::iq::groups::{
    AddressingMode, GroupAppealStatus, MemberAddMode, MemberLinkMode, MemberShareHistoryMode,
    ParticipantType,
};
use wacore::stanza::groups::{
    GroupHistorySentState, GroupParticipantType, MembershipRequestMethod,
};
use wamux_proto::v1 as pb;

use crate::group_enums::{
    addressing_mode_of, appeal_status_of, history_sent_state_of, member_add_mode_of,
    member_add_mode_token_of, member_link_mode_of, member_share_history_mode_of,
    membership_request_method_of, notification_participant_type_of, participant_type_of,
};

#[test]
fn every_addressing_mode_maps() {
    assert_eq!(
        addressing_mode_of(AddressingMode::Pn),
        pb::GroupAddressingMode::Pn
    );
    assert_eq!(
        addressing_mode_of(AddressingMode::Lid),
        pb::GroupAddressingMode::Lid
    );
}

#[test]
fn every_participant_type_maps() {
    let cases = [
        (ParticipantType::Member, pb::GroupParticipantType::Member),
        (ParticipantType::Admin, pb::GroupParticipantType::Admin),
        (
            ParticipantType::SuperAdmin,
            pb::GroupParticipantType::Superadmin,
        ),
    ];
    for (library, wire) in cases {
        assert_eq!(participant_type_of(library), wire, "{wire:?}");
    }
}

#[test]
fn every_member_add_mode_maps() {
    assert_eq!(
        member_add_mode_of(MemberAddMode::AdminAdd),
        pb::GroupMemberAddMode::AdminAdd
    );
    assert_eq!(
        member_add_mode_of(MemberAddMode::AllMemberAdd),
        pb::GroupMemberAddMode::AllMemberAdd
    );
}

#[test]
fn every_member_link_mode_maps() {
    assert_eq!(
        member_link_mode_of(MemberLinkMode::AdminLink),
        pb::GroupMemberLinkMode::AdminLink
    );
    assert_eq!(
        member_link_mode_of(MemberLinkMode::AllMemberLink),
        pb::GroupMemberLinkMode::AllMemberLink
    );
}

#[test]
fn every_member_share_history_mode_maps() {
    assert_eq!(
        member_share_history_mode_of(MemberShareHistoryMode::AdminShare),
        pb::GroupMemberShareHistoryMode::AdminShare
    );
    assert_eq!(
        member_share_history_mode_of(MemberShareHistoryMode::AllMemberShare),
        pb::GroupMemberShareHistoryMode::AllMemberShare
    );
}

#[test]
fn every_appeal_status_maps() {
    let cases = [
        (GroupAppealStatus::Approved, pb::GroupAppealStatus::Approved),
        (GroupAppealStatus::InReview, pb::GroupAppealStatus::InReview),
        (GroupAppealStatus::NoAppeal, pb::GroupAppealStatus::NoAppeal),
        (GroupAppealStatus::Rejected, pb::GroupAppealStatus::Rejected),
    ];
    for (library, wire) in cases {
        assert_eq!(appeal_status_of(library), wire, "{wire:?}");
    }
}

// #133: a notification's `participant` is the metadata's `member`, one role.
#[test]
fn every_notification_participant_type_maps() {
    let cases = [
        (
            GroupParticipantType::Participant,
            pb::GroupParticipantType::Member,
        ),
        (GroupParticipantType::Admin, pb::GroupParticipantType::Admin),
        (
            GroupParticipantType::SuperAdmin,
            pb::GroupParticipantType::Superadmin,
        ),
    ];
    for (library, wire) in cases {
        assert_eq!(notification_participant_type_of(&library), wire, "{wire:?}");
    }
}

#[test]
fn every_membership_request_method_maps() {
    let cases = [
        (
            MembershipRequestMethod::InviteLink,
            pb::MembershipRequestMethod::InviteLink,
        ),
        (
            MembershipRequestMethod::LinkedGroupJoin,
            pb::MembershipRequestMethod::LinkedGroupJoin,
        ),
        (
            MembershipRequestMethod::NonAdminAdd,
            pb::MembershipRequestMethod::NonAdminAdd,
        ),
    ];
    for (library, wire) in cases {
        assert_eq!(membership_request_method_of(&library), wire, "{wire:?}");
    }
}

#[test]
fn every_history_sent_state_maps() {
    let cases = [
        (
            GroupHistorySentState::HistoryNotSent,
            pb::GroupHistorySentState::HistoryNotSent,
        ),
        (
            GroupHistorySentState::HistorySent,
            pb::GroupHistorySentState::HistorySent,
        ),
        (
            GroupHistorySentState::NoticeSent,
            pb::GroupHistorySentState::NoticeSent,
        ),
    ];
    for (library, wire) in cases {
        assert_eq!(history_sent_state_of(library), wire, "{wire:?}");
    }
}

#[test]
fn a_named_member_add_mode_token_maps_with_no_raw() {
    assert_eq!(
        member_add_mode_token_of("admin_add"),
        (pb::GroupMemberAddMode::AdminAdd, String::new())
    );
    assert_eq!(
        member_add_mode_token_of("all_member_add"),
        (pb::GroupMemberAddMode::AllMemberAdd, String::new())
    );
}

/// The match is exact, like the library's: a case change is not a mode.
#[test]
fn an_unnamed_member_add_mode_token_keeps_it() {
    assert_eq!(
        member_add_mode_token_of("ADMIN_ADD"),
        (pb::GroupMemberAddMode::Unknown, "ADMIN_ADD".to_string())
    );
}

/// The library hands over "" when the node had no content: nothing was said.
#[test]
fn an_empty_member_add_mode_token_is_unspecified() {
    assert_eq!(
        member_add_mode_token_of(""),
        (pb::GroupMemberAddMode::Unspecified, String::new())
    );
}
