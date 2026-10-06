//! A jid field on the wire (#120): how a request's `pb::Jid` becomes a `Jid`,
//! and how a jid the core relays out becomes a `pb::Jid`.

use wamux_proto::v1 as pb;

use crate::{Jid, NewsletterJid, WamuxError, relay_jid};

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

fn plain(value: &str) -> pb::Jid {
    pb::Jid {
        value: value.to_string(),
    }
}

#[test]
fn required_wire_list_keeps_the_order() {
    let jids = Jid::from_required_wire_list(vec![plain(LID), plain(PHONE)]).unwrap();
    assert_eq!(
        jids,
        vec![Jid::parse(LID).unwrap(), Jid::parse(PHONE).unwrap()]
    );
}

#[test]
fn required_wire_list_of_nothing_is_empty() {
    assert_eq!(
        Jid::from_required_wire_list(Vec::new()).unwrap(),
        Vec::<Jid>::new()
    );
}

/// #121: each entry of a repeated jid field is required; an empty one in the
/// middle fails the whole list rather than being dropped.
#[test]
fn required_wire_list_refuses_an_empty_entry() {
    let result = Jid::from_required_wire_list(vec![plain(PHONE), plain(""), plain(LID)]);
    assert_eq!(invalid_argument(result), "missing jid");
}

#[test]
fn required_wire_list_names_the_first_malformed_entry() {
    let result =
        Jid::from_required_wire_list(vec![plain(PHONE), plain("bad one"), plain("bad two")]);
    let message = invalid_argument(result);
    assert!(
        message.starts_with("invalid jid 'bad one': "),
        "got {message:?}"
    );
}

#[test]
fn newsletter_required_wire_parses_a_channel() {
    assert_eq!(
        NewsletterJid::from_required_wire(wire(CHANNEL)).unwrap(),
        NewsletterJid::parse(CHANNEL).unwrap()
    );
}

#[test]
fn newsletter_required_wire_unset_or_empty_is_missing_jid() {
    assert_eq!(
        invalid_argument(NewsletterJid::from_required_wire(None)),
        "missing jid"
    );
    assert_eq!(
        invalid_argument(NewsletterJid::from_required_wire(wire(""))),
        "missing jid"
    );
}

/// The three channel RPCs that refused another server keep refusing it, with
/// the message they always gave.
#[test]
fn newsletter_required_wire_refuses_another_server() {
    assert_eq!(
        invalid_argument(NewsletterJid::from_required_wire(wire(PHONE))),
        format!("'{PHONE}' is not a channel: expected a jid ending in @newsletter")
    );
}
