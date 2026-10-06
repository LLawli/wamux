//! #122: every jid an event carries is the `Jid` message, relayed verbatim,
//! and one the core does not have is an unset field. Its own file because
//! `event_mapping_tests.rs` is already past the 500-line rule.

use super::*;
use wacore::stanza::groups::GroupNotificationAction;
use wacore::types::events::{ContactUpdate, GroupUpdate};

use crate::proto::v1::event_envelope::Event as PbEvent;

const GROUP_JID: &str = "120363041234567890@g.us";
const LID_JID: &str = "169815004184633@lid";
const PHONE_JID: &str = "5511999000111@s.whatsapp.net";

fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

fn jid_of(value: &str) -> Jid {
    value.parse().expect("a test jid parses")
}

fn mapped(event: &Event) -> pb::event_envelope::Event {
    map_event(event)
        .into_iter()
        .next()
        .expect("the event maps to one payload")
}

// `group_jid` became `Jid group`: only the `_jid` suffix left the name.
#[test]
fn group_update_names_the_group_as_a_jid() {
    let event = Event::GroupUpdate(
        GroupUpdate::builder()
            .group_jid(jid_of(GROUP_JID))
            .timestamp(wacore::time::from_secs(1_717_932_000).unwrap())
            .is_lid_addressing_mode(true)
            .action(Box::new(GroupNotificationAction::Announce))
            .build(),
    );
    match mapped(&event) {
        PbEvent::Group(g) => {
            assert_eq!(g.group, wire(GROUP_JID));
            assert_eq!(g.kind, "group_update");
        }
        other => panic!("expected group update, got {other:?}"),
    }
}

// A LID stays a LID: the contact's jid is relayed as the sync action named it.
#[test]
fn contact_update_names_the_contact_as_a_jid() {
    let event = Event::ContactUpdate(
        ContactUpdate::builder()
            .jid(jid_of(LID_JID))
            .timestamp(wacore::time::from_secs(1_717_932_000).unwrap())
            .action(Box::default())
            .from_full_sync(false)
            .build(),
    );
    match mapped(&event) {
        PbEvent::Contact(c) => {
            assert_eq!(c.jid, wire(LID_JID));
            assert_eq!(c.kind, "contact_update");
        }
        other => panic!("expected contact update, got {other:?}"),
    }
}

// The echo's sender is this account's own jid, which the library may not have
// yet. That was an empty string; it is an unset Jid now, never `Jid { "" }`.
#[test]
fn an_echo_without_an_own_jid_leaves_sender_unset() {
    let msg = wa::Message {
        conversation: Some("oi".to_string()),
        ..Default::default()
    };
    let out = map_sent(pb::MessageKey::default(), PHONE_JID, "", 0, &msg);
    assert_eq!(out.chat, wire(PHONE_JID));
    assert_eq!(out.sender, None);
}

/// `InboundMessage` as an edge built before #122 decodes it: `chat` and
/// `sender` were strings under numbers 2 and 3.
#[derive(Clone, PartialEq, prost::Message)]
struct InboundMessageBefore122 {
    #[prost(string, tag = "2")]
    chat: String,
    #[prost(string, tag = "3")]
    sender: String,
}

// Why the retyped fields took new numbers: a string and a message share wire
// type 2, so under the old number an old edge would read the Jid's bytes as
// its chat. Under a new number the field is unknown to it, so it reads empty,
// which is what "this edge was not updated" should look like.
#[test]
fn an_old_client_reads_a_retyped_event_field_as_absent() {
    let msg = wa::Message {
        conversation: Some("oi".to_string()),
        ..Default::default()
    };
    let out = map_sent(pb::MessageKey::default(), GROUP_JID, PHONE_JID, 0, &msg);
    let bytes = prost::Message::encode_to_vec(&out);
    let old: InboundMessageBefore122 =
        prost::Message::decode(bytes.as_slice()).expect("an old client still decodes the event");
    assert_eq!(old.chat, "");
    assert_eq!(old.sender, "");
}
