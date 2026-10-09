//! #96: a group field of a request. Unset or empty is "missing jid" (as every
//! other required jid), a jid on another server is refused naming the value,
//! before the account is even resolved.

use wamux_proto::v1 as pb;

use crate::{GroupJid, WamuxError};

const GROUP: &str = "120363000000000068@g.us";
const PHONE: &str = "5511999999999@s.whatsapp.net";
const LID: &str = "100000000000002@lid";
const CHANNEL: &str = "120363999999999999@newsletter";

fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

fn invalid_argument<T: std::fmt::Debug>(result: Result<T, WamuxError>) -> String {
    match result {
        Err(WamuxError::InvalidArgument(message)) => message,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

#[test]
fn group_required_wire_accepts_a_group() {
    let group = GroupJid::from_required_wire(wire(GROUP)).unwrap();
    assert_eq!(group, GroupJid::parse(GROUP).unwrap());
    assert_eq!(group.as_lib().to_string(), GROUP);
}

#[test]
fn group_required_wire_unset_or_empty_is_missing_jid() {
    for jid in [None, wire("")] {
        assert_eq!(
            invalid_argument(GroupJid::from_required_wire(jid)),
            "missing jid"
        );
    }
}

#[test]
fn group_required_wire_refuses_another_server_naming_the_value() {
    for other in [PHONE, LID, CHANNEL] {
        assert_eq!(
            invalid_argument(GroupJid::from_required_wire(wire(other))),
            format!("'{other}' is not a group: expected a jid ending in @g.us")
        );
    }
}

#[test]
fn group_required_wire_keeps_the_parse_message_for_a_malformed_jid() {
    let message = invalid_argument(GroupJid::from_required_wire(wire("not a jid")));
    assert!(message.starts_with("invalid jid 'not a jid':"), "{message}");
}
