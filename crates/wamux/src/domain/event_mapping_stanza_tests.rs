//! #138: the stanza behind a logout or a ban reaches the edge whole. The
//! library kept it (`raw`) so a one-time `appeal_token` survives dispatch; the
//! core used to drop it. The lock stanza here is synthetic: no real capture of
//! an account lock exists.

use std::collections::HashMap;

use super::*;
use wacore::types::events::{ConnectFailureReason, TempBanReason};

use crate::domain::test_xml::node_of_xml;
use crate::proto::v1::event_envelope::Event as PbEvent;
use crate::proto::v1::stanza_node::Content;

fn connection_change(event: &Event) -> pb::ConnectionStateChanged {
    match map_event(event).into_iter().next() {
        Some(PbEvent::Connection(c)) => c,
        other => panic!("expected a connection change, got {other:?}"),
    }
}

fn attrs_of(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn logged_out_with(raw: Option<&str>, on_connect: bool) -> Event {
    let builder = LoggedOut::builder()
        .on_connect(on_connect)
        .reason(ConnectFailureReason::AccountLocked);
    Event::LoggedOut(Box::new(builder.maybe_raw(raw.map(node_of_xml)).build()))
}

fn ban_with(raw: Option<&str>) -> Event {
    let builder = TemporaryBan::builder()
        .code(TempBanReason::SentToTooManyPeople)
        // A duration without a direct chrono dep: the difference of two instants.
        .expire(wacore::time::from_secs(3_600).unwrap() - wacore::time::from_secs(0).unwrap());
    Event::TemporaryBan(Box::new(builder.maybe_raw(raw.map(node_of_xml)).build()))
}

#[test]
fn logged_out_relays_the_failure_stanza() {
    let event = logged_out_with(
        Some(
            r#"<failure reason="403" appeal_token="tok-123" violation_reason="spam" vt="7" logout_message_header="Conta bloqueada"/>"#,
        ),
        true,
    );
    let info = connection_change(&event)
        .logged_out
        .expect("LOGGED_OUT carries its info");
    let expected = pb::StanzaNode {
        tag: "failure".to_string(),
        attrs: attrs_of(&[
            ("reason", "403"),
            ("appeal_token", "tok-123"),
            ("violation_reason", "spam"),
            ("vt", "7"),
            ("logout_message_header", "Conta bloqueada"),
        ]),
        content: None,
    };
    assert_eq!(info.stanza, Some(expected));
}

/// A logout that came after the connection is a `<stream:error>`, and its
/// reason is a child element, not an attribute of the root.
#[test]
fn logged_out_relays_a_stream_error_with_its_child() {
    let event = logged_out_with(
        Some(r#"<stream:error code="516"><conflict type="device_removed"/></stream:error>"#),
        false,
    );
    let info = connection_change(&event)
        .logged_out
        .expect("LOGGED_OUT carries its info");
    let expected = pb::StanzaNode {
        tag: "stream:error".to_string(),
        attrs: attrs_of(&[("code", "516")]),
        content: Some(Content::Children(pb::StanzaNodeList {
            nodes: vec![pb::StanzaNode {
                tag: "conflict".to_string(),
                attrs: attrs_of(&[("type", "device_removed")]),
                content: None,
            }],
        })),
    };
    assert_eq!(info.stanza, Some(expected));
}

/// A local logout received nothing: unset, never an empty node.
#[test]
fn logged_out_without_a_stanza_leaves_it_unset() {
    let info = connection_change(&logged_out_with(None, false))
        .logged_out
        .expect("LOGGED_OUT carries its info");
    assert_eq!(info.stanza, None);
}

#[test]
fn temporary_ban_relays_the_failure_stanza() {
    let event = ban_with(Some(
        r#"<failure reason="402" code="101" expire="3600" extra="kept"/>"#,
    ));
    let ban = connection_change(&event)
        .ban
        .expect("BANNED carries its info");
    let expected = pb::StanzaNode {
        tag: "failure".to_string(),
        attrs: attrs_of(&[
            ("reason", "402"),
            ("code", "101"),
            ("expire", "3600"),
            ("extra", "kept"),
        ]),
        content: None,
    };
    assert_eq!(ban.stanza, Some(expected));
}

#[test]
fn temporary_ban_without_a_stanza_leaves_it_unset() {
    let ban = connection_change(&ban_with(None))
        .ban
        .expect("BANNED carries its info");
    assert_eq!(ban.stanza, None);
}
