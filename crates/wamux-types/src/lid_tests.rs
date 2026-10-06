//! `LidPnQuery` (#116): the query text survives as sent, the jid parses.
//! #121: the query arrives as a `pb::Jid`, and its value is the echo.

use wamux_proto::v1 as pb;

use crate::{Jid, LidPnQuery, WamuxError};

fn wire(value: &str) -> pb::Jid {
    pb::Jid {
        value: value.to_string(),
    }
}

fn invalid_argument<T: std::fmt::Debug>(result: Result<T, WamuxError>) -> String {
    match result {
        Err(WamuxError::InvalidArgument(message)) => message,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

#[test]
fn a_lid_query_keeps_the_text_as_sent() {
    let query = LidPnQuery::try_from(wire("100000000000002@lid")).unwrap();
    assert_eq!(query.query, "100000000000002@lid");
    assert_eq!(query.jid, Jid::parse("100000000000002@lid").unwrap());
}

// Decided 2026-10-05: the answer echoes the query as the caller wrote it,
// while the lookup uses the jid the library's parse gives.
#[test]
fn a_legacy_query_keeps_its_spelling_and_parses_canonical() {
    let query = LidPnQuery::try_from(wire("5511999999999@c.us")).unwrap();
    assert_eq!(query.query, "5511999999999@c.us");
    assert_eq!(query.jid.to_string(), "5511999999999@s.whatsapp.net");
}

#[test]
fn a_lid_query_refuses_an_empty_or_malformed_jid() {
    assert_eq!(
        invalid_argument(LidPnQuery::try_from(pb::Jid::default())),
        "missing jid"
    );
    let message = invalid_argument(LidPnQuery::try_from(wire("not a jid")));
    assert!(message.starts_with("invalid jid 'not a jid'"), "{message}");
}
