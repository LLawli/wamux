//! #132: every value the library names maps to its own enum value. The sets
//! are closed, so there is no UNKNOWN case to test.

use wacore::iq::groups::{
    AddressingMode, GroupAppealStatus, MemberAddMode, MemberLinkMode, MemberShareHistoryMode,
    ParticipantType,
};
use wamux_proto::v1 as pb;

use crate::group_enums::{
    addressing_mode_of, appeal_status_of, member_add_mode_of, member_link_mode_of,
    member_share_history_mode_of, participant_type_of,
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
