//! `Jid`, `GroupJid`, `NewsletterJid` (#114): what each accepts, what each
//! refuses and with which message (the messages are today's wire messages).

use wamux_proto::v1 as pb;

use crate::{GroupJid, Jid, NewsletterJid, WamuxError};

const PHONE: &str = "5511999999999@s.whatsapp.net";
const GROUP: &str = "120363001234567890@g.us";
const CHANNEL: &str = "120363999999999999@newsletter";

fn invalid_argument<T: std::fmt::Debug>(result: Result<T, WamuxError>) -> String {
    match result {
        Err(WamuxError::InvalidArgument(message)) => message,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

#[test]
fn a_phone_jid_round_trips_as_written() {
    let jid = Jid::parse(PHONE).unwrap();
    assert_eq!(jid.to_string(), PHONE);
    assert_eq!(jid.as_lib().user, "5511999999999");
}

#[test]
fn lid_and_group_jids_parse() {
    assert_eq!(
        Jid::parse("123456789012345@lid").unwrap().to_string(),
        "123456789012345@lid"
    );
    assert_eq!(Jid::parse(GROUP).unwrap().to_string(), GROUP);
}

/// Decided 2026-10-05: the library's parse is the one normalization. It reads
/// `@c.us` as a phone user (the fix for #4), so it comes out canonical.
#[test]
fn the_c_us_spelling_comes_out_canonical_through_the_library() {
    let jid = Jid::parse("5511999999999@c.us").unwrap();
    assert_eq!(jid.to_string(), PHONE);
}

#[test]
fn an_empty_jid_is_invalid_argument_empty_jid() {
    assert_eq!(invalid_argument(Jid::parse("")), "empty jid");
}

#[test]
fn a_garbage_jid_names_the_offending_value() {
    let message = invalid_argument(Jid::parse("not a jid"));
    assert!(
        message.starts_with("invalid jid 'not a jid': "),
        "got {message:?}"
    );
}

#[test]
fn a_pb_jid_converts_and_back() {
    let jid = Jid::try_from(pb::Jid {
        value: PHONE.into(),
    })
    .unwrap();
    assert_eq!(jid.to_string(), PHONE);
    assert_eq!(
        pb::Jid::from(jid),
        pb::Jid {
            value: PHONE.into()
        }
    );
}

/// The services answered an empty `pb::Jid` with "missing jid" (`require_jid`).
#[test]
fn an_empty_pb_jid_is_missing_jid() {
    assert_eq!(
        invalid_argument(Jid::try_from(pb::Jid {
            value: String::new()
        })),
        "missing jid"
    );
}

#[test]
fn a_bad_pb_jid_names_the_offending_value() {
    let message = invalid_argument(Jid::try_from(pb::Jid {
        value: "x y".into(),
    }));
    assert!(
        message.starts_with("invalid jid 'x y': "),
        "got {message:?}"
    );
}

#[test]
fn a_group_jid_needs_the_group_server() {
    assert_eq!(GroupJid::parse(GROUP).unwrap().as_jid().to_string(), GROUP);
    assert_eq!(
        invalid_argument(GroupJid::parse(PHONE)),
        format!("'{PHONE}' is not a group: expected a jid ending in @g.us")
    );
    assert_eq!(invalid_argument(GroupJid::parse("")), "empty jid");
}

#[test]
fn a_newsletter_jid_needs_the_newsletter_server() {
    assert_eq!(
        NewsletterJid::parse(CHANNEL).unwrap().as_jid().to_string(),
        CHANNEL
    );
    assert_eq!(
        invalid_argument(NewsletterJid::parse(GROUP)),
        format!("'{GROUP}' is not a channel: expected a jid ending in @newsletter")
    );
    let message = invalid_argument(NewsletterJid::parse("not a jid"));
    assert!(
        message.starts_with("invalid jid 'not a jid': "),
        "got {message:?}"
    );
}
