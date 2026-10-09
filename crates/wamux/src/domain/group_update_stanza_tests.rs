//! #146: the five group actions the library parses only in part (`create`,
//! `link`, `unlink` and the two sub-group suggestions) also cross as the whole
//! element. Every notification below is a capture from 2026-10-09 (a throwaway
//! community on the trabalho phone), sent through the binary codec and parsed by
//! the library's own parser. Each expected value is written out, never built
//! with `stanza_node_of`.

use std::collections::HashMap;
use std::str::FromStr;

use wacore::stanza::groups::{GroupNotification, GroupNotificationAction};
use wacore::types::events::GroupUpdate;
use whatsapp_rust::Jid;
use whatsapp_rust::wacore_binary::builder::NodeBuilder;

use super::group_update_of;
use crate::domain::test_xml::{node_of_xml, stanza_lines, through_the_wire};
use crate::proto::v1 as pb;
use crate::proto::v1::group_update::Action;
use pb::stanza_node::Content;

const CAPTURE: &str = include_str!("fixtures/captured-community-2026-10-09.xml");
const COMMUNITY: &str = "120363000000000146@g.us";

// Line numbers in the capture (0 based, arrival order).
const SUBGROUP_CREATE: usize = 1;
const FIRST_LINK: usize = 2;
const GENERAL_CREATE: usize = 3;
const SUB_LINK: usize = 5;
const SIBLING_LINK: usize = 6;
const SUGGESTED: usize = 10;
const SUGGESTION_CANCELLED: usize = 11;
const SUGGESTION_APPROVED: usize = 15;
const UNLINK_EXISTING: usize = 16;
const UNLINK_AS_CHILD: usize = 20;
const UNLINK_DEACTIVATED: usize = 23;

fn attrs(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn leaf(tag: &str, pairs: &[(&str, &str)]) -> pb::StanzaNode {
    pb::StanzaNode {
        tag: tag.to_string(),
        attrs: attrs(pairs),
        content: None,
    }
}

fn parent(tag: &str, pairs: &[(&str, &str)], children: Vec<pb::StanzaNode>) -> pb::StanzaNode {
    pb::StanzaNode {
        tag: tag.to_string(),
        attrs: attrs(pairs),
        content: Some(Content::Children(pb::StanzaNodeList { nodes: children })),
    }
}

fn group(jid: &str, subject: &str, created: &str) -> pb::StanzaNode {
    leaf(
        "group",
        &[("jid", jid), ("subject", subject), ("s_t", created)],
    )
}

/// The update for the one action of captured line `index`, with the
/// notification header the library handler would copy onto it.
fn update_at(index: usize) -> GroupUpdate {
    let line = stanza_lines(CAPTURE)[index];
    let node = through_the_wire(&node_of_xml(line));
    let notification =
        GroupNotification::try_from_node_ref(&node.as_node_ref()).expect("a w:gp2 notification");
    assert_eq!(notification.actions.len(), 1, "line {index}");
    update_of(notification.actions[0].clone())
}

fn update_of(action: GroupNotificationAction) -> GroupUpdate {
    GroupUpdate::builder()
        .group_jid(Jid::from_str(COMMUNITY).expect("test jid parses"))
        .timestamp(wacore::time::from_secs(1_790_000_000).expect("test instant"))
        .is_lid_addressing_mode(true)
        .action(Box::new(action))
        .build()
}

fn action_at(index: usize) -> Action {
    group_update_of(&update_at(index))
        .action
        .expect("an action case")
}

#[test]
fn captured_link_relays_the_whole_element() {
    let Action::Link(first) = action_at(FIRST_LINK) else {
        panic!("a link");
    };
    // A community just created links its subgroup and the "Geral" chat at once.
    assert_eq!(first.link_type, "sub_group");
    assert_eq!(
        first.stanza,
        Some(parent(
            "link",
            &[("link_type", "sub_group")],
            vec![
                group("120363000000000147@g.us", "wamux capture 146", "1791553750"),
                group("120363000000000148@g.us", "Geral", "1791553750"),
            ]
        ))
    );
    let Action::Link(sibling) = action_at(SIBLING_LINK) else {
        panic!("a link");
    };
    assert_eq!(sibling.link_type, "sibling_group");
    assert_eq!(
        sibling.stanza,
        Some(parent(
            "link",
            &[("link_type", "sibling_group")],
            vec![group(
                "120363000000000149@g.us",
                "wamux sub 146",
                "1791553766"
            )]
        ))
    );
    let Action::Link(one) = action_at(SUB_LINK) else {
        panic!("a link");
    };
    assert_eq!(one.link_type, "sub_group");
}

#[test]
fn captured_unlink_relays_the_whole_element() {
    let cases = [
        (
            UNLINK_EXISTING,
            "sub_group",
            "unlink_group",
            group(
                "120363000000000150@g.us",
                "wamux existing 146",
                "1771933394",
            ),
        ),
        (
            UNLINK_AS_CHILD,
            "parent_group",
            "unlink_group",
            group(COMMUNITY, "wamux capture 146", "1791553750"),
        ),
        (
            UNLINK_DEACTIVATED,
            "sub_group",
            "deactivate_group",
            group("120363000000000148@g.us", "Geral", "1791553750"),
        ),
    ];
    for (index, unlink_type, reason, linked) in cases {
        let Action::Unlink(unlinked) = action_at(index) else {
            panic!("line {index}: an unlink");
        };
        assert_eq!(unlinked.unlink_type, unlink_type, "line {index}");
        assert_eq!(
            unlinked.unlink_reason.as_deref(),
            Some(reason),
            "line {index}"
        );
        let want = parent(
            "unlink",
            &[("unlink_type", unlink_type), ("unlink_reason", reason)],
            vec![linked],
        );
        assert_eq!(unlinked.stanza, Some(want), "line {index}");
    }
}

#[test]
fn captured_suggestions_relay_the_whole_element() {
    let Action::CreatedSubGroupSuggestion(created) = action_at(SUGGESTED) else {
        panic!("a created suggestion");
    };
    let suggestion = parent(
        "sub_group_suggestion",
        &[
            ("jid", "120363000000000150@g.us"),
            ("creator", "100000000000001@lid"),
            ("creation", "1791554303"),
            ("creator_pn", "5511900000001@s.whatsapp.net"),
        ],
        vec![
            token("is_existing_group", "true"),
            token("participant_count", "1"),
            text("subject", "wamux existing 146"),
            parent(
                "description",
                &[],
                vec![text("body", "wamux existing group description")],
            ),
        ],
    );
    assert_eq!(
        created.stanza,
        Some(parent(
            "created_sub_group_suggestion",
            &[],
            vec![suggestion]
        ))
    );
    for (index, reason) in [
        (SUGGESTION_CANCELLED, "cancelled"),
        (SUGGESTION_APPROVED, "approved"),
    ] {
        let Action::RevokedSubGroupSuggestions(revoked) = action_at(index) else {
            panic!("line {index}: a revoked suggestion");
        };
        let ended = leaf(
            "sub_group_suggestion",
            &[
                ("jid", "120363000000000150@g.us"),
                ("reason", reason),
                ("creator", "100000000000001@lid"),
                ("creator_pn", "5511900000001@s.whatsapp.net"),
            ],
        );
        let want = parent("revoked_sub_group_suggestions", &[], vec![ended]);
        assert_eq!(revoked.stanza, Some(want), "line {index}");
    }
}

/// Content that is a protocol token ("true", "1") comes back from the codec as
/// the library's `String`, which crosses as `Text`: the relay keeps the library's
/// own distinction between the two shapes (#138).
fn token(tag: &str, value: &str) -> pb::StanzaNode {
    pb::StanzaNode {
        tag: tag.to_string(),
        attrs: HashMap::new(),
        content: Some(Content::Text(value.to_string())),
    }
}

/// Text content as the codec returns it: bytes, since only protocol tokens
/// survive as strings (the same reason the other capture tests go through it).
fn text(tag: &str, value: &str) -> pb::StanzaNode {
    pb::StanzaNode {
        tag: tag.to_string(),
        attrs: HashMap::new(),
        content: Some(Content::Bytes(value.as_bytes().to_vec())),
    }
}

/// The element holds what the typed metadata drops: a subgroup's
/// `<linked_parent>`, the "Geral" chat's `<general_chat/>`.
#[test]
fn captured_create_relays_the_element_beside_the_metadata() {
    let Action::Create(subgroup) = action_at(SUBGROUP_CREATE) else {
        panic!("a create");
    };
    assert!(subgroup.metadata.is_some(), "the library parsed the group");
    let stanza = subgroup.stanza.expect("the whole <create>");
    assert_eq!(stanza.tag, "create");
    assert_eq!(stanza.attrs, attrs(&[("type", "new")]));
    let group_children = children_of(&children_of(&stanza)[0]);
    let tags: Vec<&str> = group_children.iter().map(|n| n.tag.as_str()).collect();
    assert_eq!(
        tags,
        [
            "linked_parent",
            "description",
            "member_share_group_history_mode",
            "default_sub_group",
            "member_add_mode",
            "announcement",
            "participant",
            "incognito",
        ]
    );
    assert_eq!(
        group_children[0],
        leaf("linked_parent", &[("jid", COMMUNITY)])
    );

    let Action::Create(general) = action_at(GENERAL_CREATE) else {
        panic!("a create");
    };
    let general_group = &children_of(&general.stanza.expect("the whole <create>"))[0];
    assert!(
        children_of(general_group).contains(&leaf("general_chat", &[])),
        "{general_group:?}"
    );
}

fn children_of(node: &pb::StanzaNode) -> Vec<pb::StanzaNode> {
    match &node.content {
        Some(Content::Children(list)) => list.nodes.clone(),
        other => panic!("expected children, got {other:?}"),
    }
}

/// A `<create>` the group parser rejects still crosses: `metadata` is unset
/// (#133) and the element is all the edge gets.
#[test]
fn a_create_the_parser_rejects_still_relays_the_element() {
    let raw = NodeBuilder::new("create")
        .attr("type", "new")
        .children([NodeBuilder::new("linked_parent")
            .attr("jid", COMMUNITY)
            .build()])
        .build();
    let update = update_of(GroupNotificationAction::Create { raw });
    assert_eq!(
        group_update_of(&update).action,
        Some(Action::Create(pb::GroupCreated {
            metadata: None,
            stanza: Some(parent(
                "create",
                &[("type", "new")],
                vec![leaf("linked_parent", &[("jid", COMMUNITY)])]
            )),
        }))
    );
}

#[test]
fn a_bare_suggestion_element_relays_as_a_leaf() {
    let update = update_of(GroupNotificationAction::RevokedSubGroupSuggestions {
        raw: NodeBuilder::new("revoked_sub_group_suggestions").build(),
    });
    assert_eq!(
        group_update_of(&update).action,
        Some(Action::RevokedSubGroupSuggestions(
            pb::GroupSubGroupSuggestionsRevoked {
                stanza: Some(leaf("revoked_sub_group_suggestions", &[])),
            }
        ))
    );
}
