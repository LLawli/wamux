//! #142: a library jid crosses as its `Display`, verbatim, by the same rule as
//! `relay_jid` (#120): the one helper every mapping uses.

use std::str::FromStr;

use wacore_binary::Jid as LibJid;
use wamux_proto::v1 as pb;

use crate::jid::{relay_jid, relay_lib_jid, relay_optional_lib_jid};

/// One jid per shape the mappings relay: a phone user's device, a hidden
/// identity, a group, a channel.
const SHAPES: [&str; 4] = [
    "5511900000001:3@s.whatsapp.net",
    "100000000000001@lid",
    "120363000000000142@g.us",
    "120363000000000143@newsletter",
];

fn lib(value: &str) -> LibJid {
    LibJid::from_str(value).expect("test jid parses")
}

fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

#[test]
fn a_lib_jid_relays_as_its_display() {
    for value in SHAPES {
        assert_eq!(relay_lib_jid(&lib(value)), wire(value), "{value}");
    }
}

#[test]
fn a_lib_jid_is_the_same_rule_as_relay_jid() {
    for value in SHAPES {
        let jid = lib(value);
        assert_eq!(relay_lib_jid(&jid), relay_jid(jid.to_string()), "{value}");
    }
}

// The library prints a jid with no user as its server, so a library jid never
// crosses unset: replacing a `Some(pb::Jid { .. })` literal with the helper
// changes nothing on the wire.
#[test]
fn a_lib_jid_with_an_empty_user_relays_its_server() {
    assert_eq!(relay_lib_jid(&LibJid::default()), wire("s.whatsapp.net"));
}

#[test]
fn an_absent_lib_jid_is_unset() {
    assert_eq!(relay_optional_lib_jid(None), None);
    for value in SHAPES {
        let jid = lib(value);
        assert_eq!(
            relay_optional_lib_jid(Some(&jid)),
            relay_lib_jid(&jid),
            "{value}"
        );
    }
}
