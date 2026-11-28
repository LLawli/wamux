//! #133: the typed projection of a library `GroupUpdate`, asserted as the whole
//! message. Three layers:
//! - the 16 `w:gp2` notifications captured live on 2026-10-07, sent through the
//!   binary codec and parsed by the library's own parser, as in production;
//! - a table with one hand-built value per library action (44), every field
//!   away from its default, for the actions nobody can provoke from two
//!   personal accounts (communities, suspension, CAPI, ...), in
//!   `group_update_action_tests.rs`;
//! - the notification header and the `<create>` metadata.

use std::str::FromStr;

use wacore::protocol::ProtocolNode;
use wacore::stanza::groups::{GroupNotification, GroupNotificationAction};
use wacore::types::events::GroupUpdate;
use whatsapp_rust::Jid;
use whatsapp_rust::wacore_binary::builder::NodeBuilder;

use super::*;
use crate::domain::group_metadata::group_metadata_of;
use crate::domain::test_xml::{attributes_sorted, node_of_xml, stanza_lines, through_the_wire};
use crate::proto::v1::group_update::Action;

const CAPTURE: &str = include_str!("fixtures/captured-gp2-2026-10-07.xml");
pub(super) const GROUP: &str = "120363000000000133@g.us";
pub(super) const OWNER_LID: &str = "100000000000001@lid";
pub(super) const OWNER_PN: &str = "5511900000001@s.whatsapp.net";
pub(super) const MEMBER_LID: &str = "100000000000002@lid";
pub(super) const MEMBER_PN: &str = "5511900000002@s.whatsapp.net";

pub(super) fn lib(value: &str) -> Jid {
    Jid::from_str(value).expect("test jid parses")
}

pub(super) fn wire(value: &str) -> Option<pb::Jid> {
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
    let Some(GroupNotificationAction::Create { raw, .. }) = notification.actions.first() else {
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

#[expect(
    clippy::too_many_lines,
    reason = "the expected value of each of the 16 captured notifications, in capture order"
)]
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
/// first. The three member modes arrive as bytes (`all_member_add` and the
/// others are not protocol tokens) and are read like strings (#205).
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
        pb::GroupMemberAddMode::AllMemberAdd as i32
    );
    assert_eq!(
        metadata.member_link_mode,
        pb::GroupMemberLinkMode::AdminLink as i32
    );
    assert_eq!(
        metadata.member_share_history_mode,
        pb::GroupMemberShareHistoryMode::AllMemberShare as i32
    );
}

/// A `<create>` with no `<group>` the parser accepts: the fact crosses, the
/// metadata stays unset rather than an empty group.
#[test]
fn create_without_a_group_leaves_metadata_unset() {
    let update = minimal(
        GroupNotificationAction::create()
            .raw(NodeBuilder::new("create").build())
            .build(),
    );
    assert_eq!(
        group_update_of(&update).action,
        Some(Action::Create(pb::GroupCreated {
            metadata: None,
            stanza: Some(bare_stanza("create")),
        }))
    );
}

/// The wire form of `NodeBuilder::new(tag).build()`: a tag and nothing else.
pub(super) fn bare_stanza(tag: &str) -> pb::StanzaNode {
    pb::StanzaNode {
        tag: tag.to_string(),
        attrs: Default::default(),
        content: None,
    }
}

pub(super) fn empty() -> pb::Empty {
    pb::Empty {}
}

/// An update carrying only `action`, the rest at the library's minimum.
pub(super) fn minimal(action: GroupNotificationAction) -> GroupUpdate {
    GroupUpdate::builder()
        .group_jid(lib(GROUP))
        .timestamp(at!(1_790_000_000))
        .is_lid_addressing_mode(false)
        .action(Box::new(action))
        .build()
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
