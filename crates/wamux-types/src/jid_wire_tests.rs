//! A jid field on the wire (#120): how a request's `pb::Jid` becomes a `Jid`,
//! and how a jid the core relays out becomes a `pb::Jid`.

use wamux_proto::v1 as pb;

use crate::{Jid, WamuxError, relay_jid};

const PHONE: &str = "5511999999999@s.whatsapp.net";

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
fn required_wire_parses_a_jid() {
    assert_eq!(
        Jid::from_required_wire(wire(PHONE)).unwrap(),
        Jid::parse(PHONE).unwrap()
    );
}

/// Decided 2026-10-05: the message `to` and `chat` already answered, now for
/// every required jid field (they answered "empty jid" as strings).
#[test]
fn required_wire_unset_is_missing_jid() {
    assert_eq!(
        invalid_argument(Jid::from_required_wire(None)),
        "missing jid"
    );
}

#[test]
fn required_wire_empty_is_missing_jid() {
    assert_eq!(
        invalid_argument(Jid::from_required_wire(wire(""))),
        "missing jid"
    );
}

#[test]
fn required_wire_malformed_names_the_value() {
    let message = invalid_argument(Jid::from_required_wire(wire("not a jid")));
    assert!(
        message.starts_with("invalid jid 'not a jid': "),
        "got {message:?}"
    );
}

#[test]
fn optional_wire_parses_a_jid() {
    assert_eq!(
        Jid::from_optional_wire(wire(PHONE)).unwrap(),
        Some(Jid::parse(PHONE).unwrap())
    );
}

#[test]
fn optional_wire_unset_is_none() {
    assert_eq!(Jid::from_optional_wire(None).unwrap(), None);
}

/// Decided 2026-10-05: a client that always fills the message with its
/// default still means "no participant", as the empty string did.
#[test]
fn optional_wire_empty_is_none() {
    assert_eq!(Jid::from_optional_wire(wire("")).unwrap(), None);
}

#[test]
fn optional_wire_malformed_is_invalid() {
    let message = invalid_argument(Jid::from_optional_wire(wire("not a jid")));
    assert!(
        message.starts_with("invalid jid 'not a jid': "),
        "got {message:?}"
    );
}

#[test]
fn relay_jid_empty_is_unset() {
    assert_eq!(relay_jid(""), None);
    assert_eq!(relay_jid(String::new()), None);
}

/// Outbound values are relayed, not parsed: a legacy spelling the library
/// handed over stays as it came, and so does a value no parser would accept.
#[test]
fn relay_jid_keeps_the_value_verbatim() {
    assert_eq!(relay_jid("5511999999999@c.us"), wire("5511999999999@c.us"));
    assert_eq!(relay_jid(PHONE.to_string()), wire(PHONE));
    assert_eq!(relay_jid("not a jid"), wire("not a jid"));
}
