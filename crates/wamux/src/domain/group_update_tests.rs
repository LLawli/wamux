//! #133: the typed projection of a library `GroupUpdate`, asserted as the whole
//! message. Three layers:
//! - the 16 `w:gp2` notifications captured live on 2026-10-07, sent through the
//!   binary codec and parsed by the library's own parser, as in production;
//! - a table with one hand-built value per library action (44), every field
//!   away from its default, for the actions nobody can provoke from two
//!   personal accounts (communities, suspension, CAPI, ...);
//! - the notification header and the `<create>` metadata.

use std::collections::BTreeSet;
use std::str::FromStr;

use wacore::protocol::ProtocolNode;
use wacore::stanza::groups::{
    GroupHistorySentState, GroupNotification, GroupNotificationAction, GroupParticipantInfo,
    GroupParticipantType, MembershipRequestMethod,
};
use wacore::types::events::GroupUpdate;
use whatsapp_rust::Jid;
use whatsapp_rust::wacore_binary::builder::NodeBuilder;

use super::*;
use crate::domain::group_metadata::group_metadata_of;
use crate::domain::test_xml::{attributes_sorted, node_of_xml, stanza_lines, through_the_wire};
use crate::proto::v1::group_update::Action;

const CAPTURE: &str = include_str!("fixtures/captured-gp2-2026-10-07.xml");
const GROUP: &str = "120363000000000133@g.us";
const OWNER_LID: &str = "100000000000001@lid";
const OWNER_PN: &str = "5511900000001@s.whatsapp.net";
const MEMBER_LID: &str = "100000000000002@lid";
const MEMBER_PN: &str = "5511900000002@s.whatsapp.net";

fn lib(value: &str) -> Jid {
    Jid::from_str(value).expect("test jid parses")
}

fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

/// The instant `seconds` after the epoch, as the library's builders take it.
/// A macro because wamux does not depend on chrono and cannot name the type.
macro_rules! at {
    ($seconds:expr) => {
        wacore::time::from_secs($seconds).expect("test instant")
    };
}

// ---------------------------------------------------------------------------
// Captured notifications
// ---------------------------------------------------------------------------

/// Every captured notification as the client parses it: through the wire,
/// then the library's `GroupNotification` parser.
fn captured() -> Vec<GroupNotification> {
    stanza_lines(CAPTURE)
        .into_iter()
        .map(|line| {
            let node = through_the_wire(&node_of_xml(line));
            GroupNotification::try_from_node_ref(&node.as_node_ref()).expect("a w:gp2 notification")
        })
        .collect()
}

/// One `GroupUpdate` per action, built the way the library's notification
/// handler builds them (`src/handlers/notification/groups.rs`, which is
/// crate-private): the header copied onto each action.
fn updates_of(notification: &GroupNotification) -> Vec<GroupUpdate> {
    let seconds = i64::try_from(notification.timestamp).expect("a sane timestamp");
    notification
        .actions
        .iter()
        .enumerate()
        .map(|(index, action)| {
            GroupUpdate::builder()
                .group_jid(notification.group_jid.clone())
                .maybe_notification_id(notification.notification_id.clone())
                .maybe_notify(notification.notify.clone())
                .maybe_offline(notification.offline.clone())
                .action_index(u32::try_from(index).expect("few actions"))
                .maybe_participant(notification.participant.clone())
                .maybe_participant_pn(notification.participant_pn.clone())
                .maybe_participant_username(notification.participant_username.clone())
                .maybe_participant_country_code(notification.participant_country_code.clone())
                .timestamp(at!(seconds))
                .is_lid_addressing_mode(notification.is_lid_addressing_mode)
                .has_incomplete_participant_information(
                    notification.has_incomplete_participant_information,
                )
                .action(Box::new(action.clone()))
                .build()
        })
        .collect()
}

/// The header every captured notification shares: caused by the owner, in a
/// LID-addressed group. `None` for `by` is the server's own change.
fn header(
    id: &str,
    seconds: i64,
    by: Option<(&str, &str, &str)>,
    action: Action,
) -> pb::GroupUpdate {
    let (participant, participant_pn, notify) = match by {
        Some((lid, pn, notify)) => (wire(lid), wire(pn), Some(notify.to_string())),
        None => (None, None, None),
    };
    pb::GroupUpdate {
        group: wire(GROUP),
        notification_id: Some(id.to_string()),
        notify,
        offline: None,
        action_index: 0,
        participant,
        participant_pn,
        participant_username: None,
        participant_country_code: None,
        timestamp: seconds * 1000,
        is_lid_addressing_mode: true,
        has_incomplete_participant_information: false,
        action: Some(action),
    }
}

const BY_OWNER: Option<(&str, &str, &str)> = Some((OWNER_LID, OWNER_PN, "Owner"));

/// A roster entry as these notifications carry it: jid and phone only; the
/// library reads the missing `type` as `participant`, which is MEMBER.
fn named(lid: &str, pn: &str) -> pb::GroupNotificationParticipant {
    pb::GroupNotificationParticipant {
        jid: wire(lid),
        phone_number: wire(pn),
        r#type: pb::GroupParticipantType::Member as i32,
        ..pb::GroupNotificationParticipant::default()
    }
}

fn change(lid: &str, pn: &str) -> pb::GroupParticipantsChange {
    pb::GroupParticipantsChange {
        participants: vec![named(lid, pn)],
        reason: None,
    }
}

fn ephemeral(seconds: u32) -> Action {
    Action::Ephemeral(pb::GroupEphemeralSettings {
        expiration_seconds: Some(seconds),
        trigger: None,
    })
}

/// The `<create>` payload as the library's own group parser reads it. Its
/// fields are pinned by `create_carries_the_group_metadata`; here it only has
/// to be the same projection the RPCs use.
fn created_metadata(notification: &GroupNotification) -> Action {
    let Some(GroupNotificationAction::Create { raw }) = notification.actions.first() else {
        panic!("not a create");
    };
    let group = raw
        .get_optional_child("group")
        .expect("<create> holds <group>");
    let response = wacore::iq::groups::GroupMetadataResponse::try_from_node(group)
        .expect("the library parses it");
    let metadata = whatsapp_rust::GroupMetadata::from(response);
    Action::Create(pb::GroupCreated {
        metadata: Some(group_metadata_of(&metadata)),
        stanza: None,
    })
}

fn expected_captures(notifications: &[GroupNotification]) -> Vec<pb::GroupUpdate> {
    vec![
        header(
            "3563024032",
            1_791_388_757,
            BY_OWNER,
            created_metadata(&notifications[0]),
        ),
        header(
            "148436923",
            1_791_388_764,
            BY_OWNER,
            Action::Subject(pb::GroupSubjectChange {
                subject: "wamux capture 133 renamed".to_string(),
                subject_owner: wire(OWNER_LID),
                subject_owner_pn: wire(OWNER_PN),
                subject_owner_username: None,
                subject_time: Some(1_791_388_764_000),
            }),
        ),
        header(
            "468538091",
            1_791_388_769,
            BY_OWNER,
            Action::Description(pb::GroupDescriptionChange {
                id: "EF71E649".to_string(),
                description: Some("capture for #133".to_string()),
            }),
        ),
        header(
            "2696110",
            1_791_388_773,
            BY_OWNER,
            Action::Announcement(pb::Empty {}),
        ),
        header(
            "2137623517",
            1_791_388_778,
            BY_OWNER,
            Action::NotAnnouncement(pb::Empty {}),
        ),
        header(
            "2286474838",
            1_791_388_782,
            BY_OWNER,
            Action::Locked(pb::GroupLockedChange { threshold: None }),
        ),
        header(
            "3530909211",
            1_791_388_787,
            BY_OWNER,
            Action::Unlocked(pb::Empty {}),
        ),
        header("256971355", 1_791_388_791, BY_OWNER, ephemeral(86_400)),
        // `<not_ephemeral/>`: the library reads it as expiration 0.
        header("273111246", 1_791_388_796, BY_OWNER, ephemeral(0)),
        header(
            "3918398580",
            1_791_388_805,
            BY_OWNER,
            Action::Promote(change(MEMBER_LID, MEMBER_PN)),
        ),
        header(
            "3425401808",
            1_791_388_809,
            BY_OWNER,
            Action::Demote(change(MEMBER_LID, MEMBER_PN)),
        ),
        header(
            "2143136731",
            1_791_388_814,
            BY_OWNER,
            Action::Remove(change(MEMBER_LID, MEMBER_PN)),
        ),
        // The re-add reaches the re-added member as a second `<create>`.
        header(
            "3312764888",
            1_791_388_819,
            BY_OWNER,
            created_metadata(&notifications[12]),
        ),
        // The owner leaves: a `<remove>` of themselves.
        header(
            "2594764884",
            1_791_388_840,
            BY_OWNER,
            Action::Remove(change(OWNER_LID, OWNER_PN)),
        ),
        // The server promotes the last member on its own: no `participant`.
        header(
            "4106332947",
            1_791_388_840,
            None,
            Action::Promote(change(MEMBER_LID, MEMBER_PN)),
        ),
        header(
            "270611905",
            1_791_388_849,
            Some((MEMBER_LID, MEMBER_PN, "Member")),
            Action::Remove(change(MEMBER_LID, MEMBER_PN)),
        ),
    ]
}

#[test]
fn captured_notifications_render_exactly_as_received() {
    for line in stanza_lines(CAPTURE) {
        let rendered = wacore::xml::DisplayableNode(&node_of_xml(line)).to_string();
        assert_eq!(attributes_sorted(&rendered), attributes_sorted(line));
    }
}

#[test]
fn every_captured_notification_maps_whole() {
    let notifications = captured();
    assert_eq!(notifications.len(), 16, "one per captured line");
    let expected = expected_captures(&notifications);
    for (index, (notification, want)) in notifications.iter().zip(expected).enumerate() {
        let updates = updates_of(notification);
        assert_eq!(
            updates.len(),
            1,
            "line {index}: one action per captured notification"
        );
        // The whole `<create>` element is pinned in group_update_stanza_tests.
        let mut got = group_update_of(&updates[0]);
        if let Some(Action::Create(created)) = got.action.as_mut() {
            created.stanza = None;
        }
        assert_eq!(got, want, "line {index}");
    }
}

/// The group a `<create>` hands over, by value: the fields the edge reads
/// first. The member modes are UNSPECIFIED today for the reason #132 pinned
/// (the library at the pin drops them when they arrive as bytes, upstream
/// #1651): the server did send `all_member_add` here.
#[test]
fn create_carries_the_group_metadata() {
    let notifications = captured();
    let Some(Action::Create(created)) = group_update_of(&updates_of(&notifications[0])[0]).action
    else {
        panic!("a create");
    };
    let metadata = created.metadata.expect("the library parsed the group");
    assert_eq!(metadata.id, wire(GROUP));
    assert_eq!(metadata.subject.as_deref(), Some("wamux capture 133"));
    assert_eq!(metadata.creator, wire(OWNER_LID));
    assert_eq!(metadata.creator_pn, wire(OWNER_PN));
    assert_eq!(metadata.creation_time, Some(1_791_388_757_000));
    assert_eq!(metadata.size, Some(2));
    let members: Vec<_> = metadata
        .participants
        .iter()
        .map(|p| p.jid.clone())
        .collect();
    assert_eq!(members, [wire(MEMBER_LID), wire(OWNER_LID)]);
    assert_eq!(
        metadata.member_add_mode,
        pb::GroupMemberAddMode::Unspecified as i32
    );
}

/// A `<create>` with no `<group>` the parser accepts: the fact crosses, the
/// metadata stays unset rather than an empty group.
#[test]
fn create_without_a_group_leaves_metadata_unset() {
    let update = minimal(GroupNotificationAction::Create {
        raw: NodeBuilder::new("create").build(),
    });
    assert_eq!(
        group_update_of(&update).action,
        Some(Action::Create(pb::GroupCreated {
            metadata: None,
            stanza: Some(bare_stanza("create")),
        }))
    );
}

// ---------------------------------------------------------------------------
// Every library action, hand-built
// ---------------------------------------------------------------------------

/// A participant with every field the notification can carry.
fn full_info() -> GroupParticipantInfo {
    GroupParticipantInfo {
        jid: lib(MEMBER_LID),
        phone_number: Some(lib(MEMBER_PN)),
        display_name: Some("+55∙∙∙∙∙∙∙∙∙02".to_string()),
        r#type: Some(GroupParticipantType::Admin),
        lid: Some(lib(MEMBER_LID)),
        username: Some("fulano".to_string()),
        join_time: Some(1_790_000_000),
        group_history_sent_state: Some(GroupHistorySentState::HistorySent),
    }
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
    GroupParticipantInfo {
        jid: lib(MEMBER_LID),
        phone_number: Some(lib(MEMBER_PN)),
        display_name: None,
        r#type: None,
        lid: None,
        username: Some("fulano".to_string()),
        join_time: None,
        group_history_sent_state: None,
    }
}

fn requested_on_the_wire() -> pb::GroupNotificationParticipant {
    pb::GroupNotificationParticipant {
        jid: wire(MEMBER_LID),
        phone_number: wire(MEMBER_PN),
        username: Some("fulano".to_string()),
        ..pb::GroupNotificationParticipant::default()
    }
}

/// The wire form of `NodeBuilder::new(tag).build()`: a tag and nothing else.
fn bare_stanza(tag: &str) -> pb::StanzaNode {
    pb::StanzaNode {
        tag: tag.to_string(),
        attrs: Default::default(),
        content: None,
    }
}

fn empty() -> pb::Empty {
    pb::Empty {}
}

fn participant_actions() -> Vec<(GroupNotificationAction, Action)> {
    use GroupNotificationAction as G;
    vec![
        (
            G::Add {
                participants: people(),
                reason: reason(),
            },
            Action::Add(people_change(Some("invite"))),
        ),
        (
            G::Remove {
                participants: people(),
                reason: reason(),
            },
            Action::Remove(people_change(Some("invite"))),
        ),
        (
            G::Promote {
                participants: people(),
            },
            Action::Promote(people_change(None)),
        ),
        (
            G::Demote {
                participants: people(),
            },
            Action::Demote(people_change(None)),
        ),
        (
            G::Modify {
                participants: people(),
            },
            Action::Modify(people_change(None)),
        ),
        (
            G::LinkedGroupPromote {
                participants: people(),
            },
            Action::LinkedGroupPromote(people_change(None)),
        ),
        (
            G::LinkedGroupDemote {
                participants: people(),
            },
            Action::LinkedGroupDemote(people_change(None)),
        ),
    ]
}

fn metadata_actions() -> Vec<(GroupNotificationAction, Action)> {
    use GroupNotificationAction as G;
    vec![
        (
            G::Subject {
                subject: "new name".to_string(),
                subject_owner: Some(lib(OWNER_LID)),
                subject_owner_pn: Some(lib(OWNER_PN)),
                subject_owner_username: Some("owner.user".to_string()),
                subject_time: Some(1_790_000_001),
            },
            Action::Subject(pb::GroupSubjectChange {
                subject: "new name".to_string(),
                subject_owner: wire(OWNER_LID),
                subject_owner_pn: wire(OWNER_PN),
                subject_owner_username: Some("owner.user".to_string()),
                subject_time: Some(1_790_000_001_000),
            }),
        ),
        (
            G::Description {
                id: "D1".to_string(),
                description: Some("text".to_string()),
            },
            Action::Description(pb::GroupDescriptionChange {
                id: "D1".to_string(),
                description: Some("text".to_string()),
            }),
        ),
        (
            G::Locked {
                threshold: Some("2".to_string()),
            },
            Action::Locked(pb::GroupLockedChange {
                threshold: Some("2".to_string()),
            }),
        ),
        (
            G::Ephemeral {
                expiration: 604_800,
                trigger: Some(3),
            },
            Action::Ephemeral(pb::GroupEphemeralSettings {
                expiration_seconds: Some(604_800),
                trigger: Some(3),
            }),
        ),
        (
            G::Delete {
                reason: Some("admin".to_string()),
            },
            Action::Delete(pb::GroupDeleted {
                reason: Some("admin".to_string()),
            }),
        ),
        (
            G::Unknown {
                tag: "future_tag".to_string(),
            },
            Action::Unknown(pb::GroupUnknownAction {
                tag: "future_tag".to_string(),
            }),
        ),
    ]
}

fn membership_actions() -> Vec<(GroupNotificationAction, Action)> {
    use GroupNotificationAction as G;
    vec![
        (
            G::MembershipApprovalMode { enabled: true },
            Action::MembershipApprovalMode(pb::GroupMembershipApprovalMode { enabled: true }),
        ),
        (
            G::MembershipApprovalRequest {
                request_method: MembershipRequestMethod::LinkedGroupJoin,
                parent_group_jid: Some(lib("120363000000000001@g.us")),
            },
            Action::MembershipApprovalRequest(pb::GroupMembershipApprovalRequest {
                request_method: pb::MembershipRequestMethod::LinkedGroupJoin as i32,
                parent_group: wire("120363000000000001@g.us"),
            }),
        ),
        (
            G::CreatedMembershipRequests {
                request_method: MembershipRequestMethod::NonAdminAdd,
                parent_group_jid: Some(lib("120363000000000001@g.us")),
                requests: vec![requested()],
            },
            Action::CreatedMembershipRequests(pb::GroupCreatedMembershipRequests {
                request_method: pb::MembershipRequestMethod::NonAdminAdd as i32,
                parent_group: wire("120363000000000001@g.us"),
                requests: vec![requested_on_the_wire()],
            }),
        ),
        (
            G::RevokedMembershipRequests {
                participants: vec![lib(MEMBER_LID)],
            },
            Action::RevokedMembershipRequests(pb::GroupRevokedMembershipRequests {
                participants: vec![pb::Jid {
                    value: MEMBER_LID.to_string(),
                }],
            }),
        ),
        (
            G::MemberAddMode {
                mode: "admin_add".to_string(),
            },
            Action::MemberAddMode(pb::GroupMemberAddModeChange {
                mode: pb::GroupMemberAddMode::AdminAdd as i32,
                mode_raw: String::new(),
            }),
        ),
        (
            G::Invite {
                code: "AbCdEfGhIjKlMnOpQrStUv".to_string(),
            },
            Action::Invite(pb::GroupInviteJoin {
                code: "AbCdEfGhIjKlMnOpQrStUv".to_string(),
            }),
        ),
        (
            G::GrowthLocked {
                expiration: 86_400,
                lock_type: "invite".to_string(),
            },
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
            G::Link {
                link_type: "sub_group".to_string(),
                raw: NodeBuilder::new("link").build(),
            },
            Action::Link(pb::GroupLinked {
                link_type: "sub_group".to_string(),
                stanza: Some(bare_stanza("link")),
            }),
        ),
        (
            G::Unlink {
                unlink_type: "sub_group".to_string(),
                unlink_reason: Some("delete_parent".to_string()),
                raw: NodeBuilder::new("unlink").build(),
            },
            Action::Unlink(pb::GroupUnlinked {
                unlink_type: "sub_group".to_string(),
                unlink_reason: Some("delete_parent".to_string()),
                stanza: Some(bare_stanza("unlink")),
            }),
        ),
        (
            G::LimitSharingEnabled { trigger: Some(4) },
            Action::LimitSharingEnabled(pb::GroupLimitSharing { trigger: Some(4) }),
        ),
        (
            G::ChangeNumber {
                new_owner: Some(lib(OWNER_PN)),
                sub_group_suggestions: vec![lib("120363000000000002@g.us")],
            },
            Action::ChangeNumber(pb::GroupChangeNumber {
                new_owner: wire(OWNER_PN),
                sub_group_suggestions: vec![pb::Jid {
                    value: "120363000000000002@g.us".to_string(),
                }],
            }),
        ),
        (
            G::CreatedSubGroupSuggestion {
                raw: NodeBuilder::new("created_sub_group_suggestion").build(),
            },
            Action::CreatedSubGroupSuggestion(pb::GroupSubGroupSuggestionCreated {
                stanza: Some(bare_stanza("created_sub_group_suggestion")),
            }),
        ),
        (
            G::RevokedSubGroupSuggestions {
                raw: NodeBuilder::new("revoked_sub_group_suggestions").build(),
            },
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

/// Every library action but `Create`, which has its own tests above.
fn every_action() -> Vec<(GroupNotificationAction, Action)> {
    let mut all = participant_actions();
    all.extend(metadata_actions());
    all.extend(membership_actions());
    all.extend(community_actions());
    all.extend(flag_actions());
    all
}

/// An update carrying only `action`, the rest at the library's minimum.
fn minimal(action: GroupNotificationAction) -> GroupUpdate {
    GroupUpdate::builder()
        .group_jid(lib(GROUP))
        .timestamp(at!(1_790_000_000))
        .is_lid_addressing_mode(false)
        .action(Box::new(action))
        .build()
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
    let update = minimal(GroupNotificationAction::MemberAddMode {
        mode: "everyone".to_string(),
    });
    assert_eq!(
        group_update_of(&update).action,
        Some(Action::MemberAddMode(pb::GroupMemberAddModeChange {
            mode: pb::GroupMemberAddMode::Unknown as i32,
            mode_raw: "everyone".to_string(),
        }))
    );
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

#[test]
fn group_update_notification_fields_map() {
    let update = GroupUpdate::builder()
        .group_jid(lib(GROUP))
        .notification_id("N1".to_string())
        .notify("Owner".to_string())
        .offline("1".to_string())
        .action_index(2)
        .participant(lib(OWNER_LID))
        .participant_pn(lib(OWNER_PN))
        .participant_username("owner.user".to_string())
        .participant_country_code("BR".to_string())
        .timestamp(at!(1_790_000_000))
        .is_lid_addressing_mode(true)
        .has_incomplete_participant_information(true)
        .action(Box::new(GroupNotificationAction::Unlocked))
        .build();
    let expected = pb::GroupUpdate {
        group: wire(GROUP),
        notification_id: Some("N1".to_string()),
        notify: Some("Owner".to_string()),
        offline: Some("1".to_string()),
        action_index: 2,
        participant: wire(OWNER_LID),
        participant_pn: wire(OWNER_PN),
        participant_username: Some("owner.user".to_string()),
        participant_country_code: Some("BR".to_string()),
        timestamp: 1_790_000_000_000,
        is_lid_addressing_mode: true,
        has_incomplete_participant_information: true,
        action: Some(Action::Unlocked(empty())),
    };
    assert_eq!(group_update_of(&update), expected);
}

#[test]
fn group_update_absent_fields_stay_unset() {
    let expected = pb::GroupUpdate {
        group: wire(GROUP),
        timestamp: 1_790_000_000_000,
        action: Some(Action::Announcement(empty())),
        ..pb::GroupUpdate::default()
    };
    assert_eq!(
        group_update_of(&minimal(GroupNotificationAction::Announce)),
        expected
    );
}
