//! `blocklist` (#197): which jids a block accepts, how the library's refusals
//! cross the wire, and how a server entry renders.

use whatsapp_rust::features::{BlockingError, BlocklistEntry};

use super::{blocking_err, blocklist_entry_of, require_user_jid};
use crate::error::WamuxError;
use crate::proto::v1 as pb;
use wamux_types::Jid;

fn jid(value: &str) -> Jid {
    Jid::parse(value).expect("well-formed test jid")
}

fn lib_jid(value: &str) -> whatsapp_rust::Jid {
    value.parse().expect("well-formed test jid")
}

#[test]
fn user_jids_pass_the_block_check() {
    for value in [
        "5511999000111@s.whatsapp.net",
        "5511999000111:7@s.whatsapp.net",
        // The library parses the legacy spelling as a phone user.
        "5511999000111@c.us",
        "169815004184633@lid",
    ] {
        assert!(require_user_jid(&jid(value)).is_ok(), "{value} is a user");
    }
}

// Only a phone user or a LID has a blocklist entry. `@c.us` is not here: the
// library's parse already reads it as a phone user.
#[test]
fn non_user_jids_are_invalid_argument() {
    for value in [
        "13135550002@bot",
        "120363000000000001@g.us",
        "120363000000000002@newsletter",
        "status@broadcast",
    ] {
        match require_user_jid(&jid(value)) {
            Err(WamuxError::InvalidArgument(reason)) => {
                assert!(reason.contains(value), "{value} named in: {reason}")
            }
            other => panic!("{value} must be InvalidArgument, got {other:?}"),
        }
    }
}

// Past the kind check, the library's `InvalidJid` can only mean the client has
// no LID<->PN pair for the jid: state the edge can fix, not a bad argument.
#[test]
fn missing_mapping_is_failed_precondition() {
    let err = blocking_err(BlockingError::InvalidJid(
        "no LID↔PN mapping for provided jid".to_string(),
    ));
    match err {
        WamuxError::FailedPrecondition(reason) => {
            assert!(
                reason.contains("no LID↔PN mapping"),
                "lib reason kept: {reason}"
            )
        }
        other => panic!("expected FailedPrecondition, got {other:?}"),
    }
}

#[test]
fn blocklist_entry_keeps_jid_and_seconds() {
    let entry = BlocklistEntry {
        jid: lib_jid("169815004184633@lid"),
        timestamp: Some(1_760_000_000),
    };
    assert_eq!(
        blocklist_entry_of(&entry),
        pb::BlocklistEntry {
            jid: Some(pb::Jid {
                value: "169815004184633@lid".to_string()
            }),
            timestamp: Some(1_760_000_000),
        }
    );
}

#[test]
fn blocklist_entry_without_t_leaves_timestamp_unset() {
    let entry = BlocklistEntry {
        jid: lib_jid("5511999000111@s.whatsapp.net"),
        timestamp: None,
    };
    let rendered = blocklist_entry_of(&entry);
    assert_eq!(rendered.timestamp, None);
    assert_eq!(
        rendered.jid.map(|j| j.value).as_deref(),
        Some("5511999000111@s.whatsapp.net")
    );
}

// A `t` past i64 is not a time the edge can use; unset says "no time", where a
// wrapped negative number would lie.
#[test]
fn blocklist_entry_past_i64_leaves_timestamp_unset() {
    let entry = BlocklistEntry {
        jid: lib_jid("169815004184633@lid"),
        timestamp: Some(u64::MAX),
    };
    assert_eq!(blocklist_entry_of(&entry).timestamp, None);
}
