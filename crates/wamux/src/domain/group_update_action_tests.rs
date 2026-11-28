//! #133: one hand-built value per library `GroupNotificationAction` (44), every
//! field away from its default, for the actions nobody can provoke from two
//! personal accounts (communities, suspension, CAPI, ...).

use std::collections::BTreeSet;

use wacore::stanza::groups::{
    GroupHistorySentState, GroupNotificationAction, GroupParticipantInfo, GroupParticipantType,
    MembershipRequestMethod,
};
use whatsapp_rust::wacore_binary::builder::NodeBuilder;

use super::tests::{
    MEMBER_LID, MEMBER_PN, OWNER_LID, OWNER_PN, bare_stanza, empty, lib, minimal, wire,
};
use super::*;
use crate::proto::v1::group_update::Action;

/// A participant with every field the notification can carry.
fn full_info() -> GroupParticipantInfo {
    GroupParticipantInfo::builder()
        .jid(lib(MEMBER_LID))
        .phone_number(lib(MEMBER_PN))
        .display_name("+55∙∙∙∙∙∙∙∙∙02".to_string())
        .r#type(GroupParticipantType::Admin)
        .lid(lib(MEMBER_LID))
        .username("fulano".to_string())
        .join_time(1_790_000_000)
        .group_history_sent_state(GroupHistorySentState::HistorySent)
        .build()
}

fn full_info_on_the_wire() -> pb::GroupNotificationParticipant {
    pb::GroupNotificationParticipant {
        jid: wire(MEMBER_LID),
        phone_number: wire(MEMBER_PN),
        display_name: Some("+55∙∙∙∙∙∙∙∙∙02".to_string()),
        r#type: pb::GroupParticipantType::Admin as i32,
        lid: wire(MEMBER_LID),
        username: Some("fulano".to_string()),
        join_time: Some(1_790_000_000_000),
        group_history_sent_state: pb::GroupHistorySentState::HistorySent as i32,
    }
}

fn people() -> Vec<GroupParticipantInfo> {
    vec![full_info()]
}

fn people_change(reason: Option<&str>) -> pb::GroupParticipantsChange {
    pb::GroupParticipantsChange {
        participants: vec![full_info_on_the_wire()],
        reason: reason.map(str::to_string),
    }
}

fn reason() -> Option<String> {
    Some("invite".to_string())
}

/// A join request's entry: the library reads only jid, phone and username, so
/// the type stays UNSPECIFIED.
fn requested() -> GroupParticipantInfo {
    GroupParticipantInfo::builder()
        .jid(lib(MEMBER_LID))
        .phone_number(lib(MEMBER_PN))
        .username("fulano".to_string())
        .build()
}

fn requested_on_the_wire() -> pb::GroupNotificationParticipant {
    pb::GroupNotificationParticipant {
        jid: wire(MEMBER_LID),
        phone_number: wire(MEMBER_PN),
        username: Some("fulano".to_string()),
        ..pb::GroupNotificationParticipant::default()
    }
}

fn participant_actions() -> Vec<(GroupNotificationAction, Action)> {
    use GroupNotificationAction as G;
    vec![
        (
            G::add()
                .participants(people())
                .maybe_reason(reason())
                .build(),
            Action::Add(people_change(Some("invite"))),
        ),
        (
            G::remove()
                .participants(people())
                .maybe_reason(reason())
                .build(),
            Action::Remove(people_change(Some("invite"))),
        ),
        (
            G::promote().participants(people()).build(),
            Action::Promote(people_change(None)),
        ),
        (
            G::demote().participants(people()).build(),
            Action::Demote(people_change(None)),
        ),
        (
            G::modify().participants(people()).build(),
            Action::Modify(people_change(None)),
        ),
        (
            G::linked_group_promote().participants(people()).build(),
            Action::LinkedGroupPromote(people_change(None)),
        ),
        (
            G::linked_group_demote().participants(people()).build(),
            Action::LinkedGroupDemote(people_change(None)),
        ),
    ]
}

fn metadata_actions() -> Vec<(GroupNotificationAction, Action)> {
    use GroupNotificationAction as G;
    vec![
        (
            G::subject()
                .subject("new name".to_string())
                .maybe_subject_owner(Some(lib(OWNER_LID)))
                .maybe_subject_owner_pn(Some(lib(OWNER_PN)))
                .maybe_subject_owner_username(Some("owner.user".to_string()))
                .maybe_subject_time(Some(1_790_000_001))
                .build(),
            Action::Subject(pb::GroupSubjectChange {
                subject: "new name".to_string(),
                subject_owner: wire(OWNER_LID),
                subject_owner_pn: wire(OWNER_PN),
                subject_owner_username: Some("owner.user".to_string()),
                subject_time: Some(1_790_000_001_000),
            }),
        ),
        (
            G::description()
                .id("D1".to_string())
                .maybe_description(Some("text".to_string()))
                .build(),
            Action::Description(pb::GroupDescriptionChange {
                id: "D1".to_string(),
                description: Some("text".to_string()),
            }),
        ),
        (
            G::locked().maybe_threshold(Some("2".to_string())).build(),
            Action::Locked(pb::GroupLockedChange {
                threshold: Some("2".to_string()),
            }),
        ),
        (
            G::ephemeral()
                .expiration(604_800)
                .maybe_trigger(Some(3))
                .build(),
            Action::Ephemeral(pb::GroupEphemeralSettings {
                expiration_seconds: Some(604_800),
                trigger: Some(3),
            }),
        ),
        (
            G::delete().maybe_reason(Some("admin".to_string())).build(),
            Action::Delete(pb::GroupDeleted {
                reason: Some("admin".to_string()),
            }),
        ),
        (
            G::unknown().tag("future_tag".to_string()).build(),
            Action::Unknown(pb::GroupUnknownAction {
                tag: "future_tag".to_string(),
            }),
        ),
    ]
}

#[expect(
    clippy::too_many_lines,
    reason = "fixture table: one hand-built value per membership GroupNotificationAction and its wire case"
)]
fn membership_actions() -> Vec<(GroupNotificationAction, Action)> {
    use GroupNotificationAction as G;
    vec![
        (
            G::membership_approval_mode().enabled(true).build(),
            Action::MembershipApprovalMode(pb::GroupMembershipApprovalMode { enabled: true }),
        ),
        (
            G::membership_approval_request()
                .request_method(MembershipRequestMethod::LinkedGroupJoin)
                .maybe_parent_group_jid(Some(lib("120363000000000001@g.us")))
                .build(),
            Action::MembershipApprovalRequest(pb::GroupMembershipApprovalRequest {
                request_method: pb::MembershipRequestMethod::LinkedGroupJoin as i32,
                parent_group: wire("120363000000000001@g.us"),
            }),
        ),
        (
            G::created_membership_requests()
                .request_method(MembershipRequestMethod::NonAdminAdd)
                .maybe_parent_group_jid(Some(lib("120363000000000001@g.us")))
                .requests(vec![requested()])
                .build(),
            Action::CreatedMembershipRequests(pb::GroupCreatedMembershipRequests {
                request_method: pb::MembershipRequestMethod::NonAdminAdd as i32,
                parent_group: wire("120363000000000001@g.us"),
                requests: vec![requested_on_the_wire()],
            }),
        ),
        (
            G::revoked_membership_requests()
                .participants(vec![lib(MEMBER_LID)])
                .build(),
            Action::RevokedMembershipRequests(pb::GroupRevokedMembershipRequests {
                participants: vec![pb::Jid {
                    value: MEMBER_LID.to_string(),
                }],
            }),
        ),
        (
            G::member_add_mode().mode("admin_add".to_string()).build(),
            Action::MemberAddMode(pb::GroupMemberAddModeChange {
                mode: pb::GroupMemberAddMode::AdminAdd as i32,
                mode_raw: String::new(),
            }),
        ),
        (
            G::invite()
                .code("AbCdEfGhIjKlMnOpQrStUv".to_string())
                .build(),
            Action::Invite(pb::GroupInviteJoin {
                code: "AbCdEfGhIjKlMnOpQrStUv".to_string(),
            }),
        ),
        (
            G::growth_locked()
                .expiration(86_400)
                .lock_type("invite".to_string())
                .build(),
            // Unit not verified (#132): verbatim.
            Action::GrowthLocked(pb::GrowthLockInfo {
                lock_type: "invite".to_string(),
                expiration: 86_400,
            }),
        ),
    ]
}

fn community_actions() -> Vec<(GroupNotificationAction, Action)> {
    use GroupNotificationAction as G;
    vec![
        (
            G::link()
                .link_type("sub_group".to_string())
                .raw(NodeBuilder::new("link").build())
                .build(),
            Action::Link(pb::GroupLinked {
                link_type: "sub_group".to_string(),
                stanza: Some(bare_stanza("link")),
            }),
        ),
        (
            G::unlink()
                .unlink_type("sub_group".to_string())
                .maybe_unlink_reason(Some("delete_parent".to_string()))
                .raw(NodeBuilder::new("unlink").build())
                .build(),
            Action::Unlink(pb::GroupUnlinked {
                unlink_type: "sub_group".to_string(),
                unlink_reason: Some("delete_parent".to_string()),
                stanza: Some(bare_stanza("unlink")),
            }),
        ),
        (
            G::limit_sharing_enabled().maybe_trigger(Some(4)).build(),
            Action::LimitSharingEnabled(pb::GroupLimitSharing { trigger: Some(4) }),
        ),
        (
            G::change_number()
                .maybe_new_owner(Some(lib(OWNER_PN)))
                .sub_group_suggestions(vec![lib("120363000000000002@g.us")])
                .build(),
            Action::ChangeNumber(pb::GroupChangeNumber {
                new_owner: wire(OWNER_PN),
                sub_group_suggestions: vec![pb::Jid {
                    value: "120363000000000002@g.us".to_string(),
                }],
            }),
        ),
        (
            G::created_sub_group_suggestion()
                .raw(NodeBuilder::new("created_sub_group_suggestion").build())
                .build(),
            Action::CreatedSubGroupSuggestion(pb::GroupSubGroupSuggestionCreated {
                stanza: Some(bare_stanza("created_sub_group_suggestion")),
            }),
        ),
        (
            G::revoked_sub_group_suggestions()
                .raw(NodeBuilder::new("revoked_sub_group_suggestions").build())
                .build(),
            Action::RevokedSubGroupSuggestions(pb::GroupSubGroupSuggestionsRevoked {
                stanza: Some(bare_stanza("revoked_sub_group_suggestions")),
            }),
        ),
    ]
}

fn flag_actions() -> Vec<(GroupNotificationAction, Action)> {
    use GroupNotificationAction as G;
    vec![
        (G::Unlocked, Action::Unlocked(empty())),
        (G::Announce, Action::Announcement(empty())),
        (G::NotAnnounce, Action::NotAnnouncement(empty())),
        (
            G::NoFrequentlyForwarded,
            Action::NoFrequentlyForwarded(empty()),
        ),
        (
            G::FrequentlyForwardedOk,
            Action::FrequentlyForwardedOk(empty()),
        ),
        (G::RevokeInvite, Action::Revoke(empty())),
        (G::GrowthUnlocked, Action::GrowthUnlocked(empty())),
        (G::Suspended, Action::Suspended(empty())),
        (G::Unsuspended, Action::Unsuspended(empty())),
        (G::AutoAddDisabled, Action::AutoAddDisabled(empty())),
        (G::IsCapiHostedGroup, Action::IsCapiHostedGroup(empty())),
        (G::GroupSafetyCheck, Action::GroupSafetyCheck(empty())),
        (G::AllowAdminReports, Action::AllowAdminReports(empty())),
        (
            G::NotAllowAdminReports,
            Action::NotAllowAdminReports(empty()),
        ),
        (G::Reports, Action::Reports(empty())),
        (
            G::AllowNonAdminSubGroupCreation,
            Action::AllowNonAdminSubGroupCreation(empty()),
        ),
        (
            G::NotAllowNonAdminSubGroupCreation,
            Action::NotAllowNonAdminSubGroupCreation(empty()),
        ),
    ]
}

/// Every library action but `Create`, which has its own tests in `group_update_tests.rs`.
fn every_action() -> Vec<(GroupNotificationAction, Action)> {
    let mut all = participant_actions();
    all.extend(metadata_actions());
    all.extend(membership_actions());
    all.extend(community_actions());
    all.extend(flag_actions());
    all
}

#[test]
fn every_library_action_maps_to_its_own_case() {
    let all = every_action();
    // 44 variants at the pin, minus `Create`. The tags must all differ, so no
    // variant stands in for another and none is missing from the table.
    let tags: BTreeSet<&str> = all.iter().map(|(action, _)| action.wire_tag()).collect();
    assert_eq!(all.len(), 43);
    assert_eq!(tags.len(), 43, "{tags:?}");
    for (action, want) in all {
        let tag = action.wire_tag().to_string();
        assert_eq!(
            group_update_of(&minimal(action)).action,
            Some(want),
            "{tag}"
        );
    }
}

/// `MemberAddMode` carries the server's token; one the library names no mode
/// for keeps it in `mode_raw`.
#[test]
fn an_unnamed_member_add_mode_keeps_its_token() {
    let update = minimal(
        GroupNotificationAction::member_add_mode()
            .mode("everyone".to_string())
            .build(),
    );
    assert_eq!(
        group_update_of(&update).action,
        Some(Action::MemberAddMode(pb::GroupMemberAddModeChange {
            mode: pb::GroupMemberAddMode::Unknown as i32,
            mode_raw: "everyone".to_string(),
        }))
    );
}
