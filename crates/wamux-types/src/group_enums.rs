//! The library's group enums as the contract's enums (#132, part 1 of #74).
//!
//! Every one of these library enums is a closed set, so each `match` has no
//! wildcard arm: a variant added upstream fails the build at the bump instead
//! of reaching the edge unnamed, and UNKNOWN is never emitted.

use wacore::iq::groups::{
    AddressingMode, GroupAppealStatus, MemberAddMode, MemberLinkMode, MemberShareHistoryMode,
    ParticipantType,
};
use wacore::stanza::groups::{
    GroupHistorySentState, GroupParticipantType, MembershipRequestMethod,
};
use wamux_proto::v1 as pb;

pub fn addressing_mode_of(mode: AddressingMode) -> pb::GroupAddressingMode {
    match mode {
        AddressingMode::Pn => pb::GroupAddressingMode::Pn,
        AddressingMode::Lid => pb::GroupAddressingMode::Lid,
    }
}

pub fn participant_type_of(participant_type: ParticipantType) -> pb::GroupParticipantType {
    match participant_type {
        ParticipantType::Member => pb::GroupParticipantType::Member,
        ParticipantType::Admin => pb::GroupParticipantType::Admin,
        ParticipantType::SuperAdmin => pb::GroupParticipantType::Superadmin,
    }
}

pub fn member_add_mode_of(mode: MemberAddMode) -> pb::GroupMemberAddMode {
    match mode {
        MemberAddMode::AdminAdd => pb::GroupMemberAddMode::AdminAdd,
        MemberAddMode::AllMemberAdd => pb::GroupMemberAddMode::AllMemberAdd,
    }
}

pub fn member_link_mode_of(mode: MemberLinkMode) -> pb::GroupMemberLinkMode {
    match mode {
        MemberLinkMode::AdminLink => pb::GroupMemberLinkMode::AdminLink,
        MemberLinkMode::AllMemberLink => pb::GroupMemberLinkMode::AllMemberLink,
    }
}

pub fn member_share_history_mode_of(
    mode: MemberShareHistoryMode,
) -> pb::GroupMemberShareHistoryMode {
    match mode {
        MemberShareHistoryMode::AdminShare => pb::GroupMemberShareHistoryMode::AdminShare,
        MemberShareHistoryMode::AllMemberShare => pb::GroupMemberShareHistoryMode::AllMemberShare,
    }
}

pub fn appeal_status_of(status: GroupAppealStatus) -> pb::GroupAppealStatus {
    match status {
        GroupAppealStatus::Approved => pb::GroupAppealStatus::Approved,
        GroupAppealStatus::InReview => pb::GroupAppealStatus::InReview,
        GroupAppealStatus::NoAppeal => pb::GroupAppealStatus::NoAppeal,
        GroupAppealStatus::Rejected => pb::GroupAppealStatus::Rejected,
    }
}

/// The role a group NOTIFICATION gives a member (#133). The server spells the
/// plain member `participant` there and `member` in the metadata answer; both
/// are the same role, so both are GROUP_PARTICIPANT_TYPE_MEMBER.
pub fn notification_participant_type_of(
    participant_type: &GroupParticipantType,
) -> pb::GroupParticipantType {
    match participant_type {
        GroupParticipantType::Participant => pb::GroupParticipantType::Member,
        GroupParticipantType::Admin => pb::GroupParticipantType::Admin,
        GroupParticipantType::SuperAdmin => pb::GroupParticipantType::Superadmin,
    }
}

pub fn membership_request_method_of(
    method: &MembershipRequestMethod,
) -> pb::MembershipRequestMethod {
    match method {
        MembershipRequestMethod::InviteLink => pb::MembershipRequestMethod::InviteLink,
        MembershipRequestMethod::LinkedGroupJoin => pb::MembershipRequestMethod::LinkedGroupJoin,
        MembershipRequestMethod::NonAdminAdd => pb::MembershipRequestMethod::NonAdminAdd,
    }
}

pub fn history_sent_state_of(state: GroupHistorySentState) -> pb::GroupHistorySentState {
    match state {
        GroupHistorySentState::HistoryNotSent => pb::GroupHistorySentState::HistoryNotSent,
        GroupHistorySentState::HistorySent => pb::GroupHistorySentState::HistorySent,
        GroupHistorySentState::NoticeSent => pb::GroupHistorySentState::NoticeSent,
    }
}

/// A `<member_add_mode>` notification's content, which the library hands over
/// as the raw string: parsed with the library's own exact match, UNKNOWN plus
/// the token when it names no mode, UNSPECIFIED when the node was empty.
pub fn member_add_mode_token_of(token: &str) -> (pb::GroupMemberAddMode, String) {
    if token.is_empty() {
        return (pb::GroupMemberAddMode::Unspecified, String::new());
    }
    match MemberAddMode::try_from(token) {
        Ok(mode) => (member_add_mode_of(mode), String::new()),
        Err(_) => (pb::GroupMemberAddMode::Unknown, token.to_string()),
    }
}
