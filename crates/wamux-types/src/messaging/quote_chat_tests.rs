//! #217: the quoted key's chat on an outgoing quote. It becomes the context's
//! `remoteJid` only when it names a chat other than the one the reply is sent
//! to, the way WhatsApp's clients send a private reply or a reply to a status:
//! a reply in the same chat carries none.

use wamux_proto::v1 as pb;

use crate::{Jid, QuotedRef, WamuxError};

const DM: &str = "5511999999999@s.whatsapp.net";
const GROUP: &str = "120363001234567890@g.us";
const AUTHOR: &str = "5511777777777@s.whatsapp.net";
const STATUS: &str = "status@broadcast";

fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

fn quote_from(chat: Option<pb::Jid>) -> Option<pb::QuoteContext> {
    Some(pb::QuoteContext {
        quoted: Some(pb::MessageKey {
            chat,
            id: "QUOTED-217".to_string(),
            from_me: false,
            participant: wire(AUTHOR),
        }),
        participant: None,
    })
}

fn remote_chat(chat: Option<pb::Jid>, destination: &str) -> Option<Jid> {
    QuotedRef::from_proto(quote_from(chat), wire(destination).as_ref())
        .expect("a valid quote")
        .expect("a quoted key is a quote")
        .remote_chat
}

#[test]
fn quoted_chat_other_than_the_destination_is_the_remote_chat() {
    assert_eq!(
        remote_chat(wire(GROUP), DM),
        Some(Jid::parse(GROUP).unwrap()),
        "a private reply names the group the quoted message is in"
    );
    assert_eq!(
        remote_chat(wire(STATUS), DM),
        Some(Jid::parse(STATUS).unwrap()),
        "a reply to a status names status@broadcast"
    );
}

#[test]
fn quoted_chat_equal_to_the_destination_is_dropped() {
    assert_eq!(remote_chat(wire(GROUP), GROUP), None);
    assert_eq!(remote_chat(wire(DM), DM), None);
    // The legacy spelling parses to the same phone jid as the destination.
    assert_eq!(remote_chat(wire("5511999999999@c.us"), DM), None);
}

#[test]
fn unset_or_empty_quoted_chat_is_none() {
    assert_eq!(remote_chat(None, DM), None);
    assert_eq!(remote_chat(wire(""), DM), None);
}

#[test]
fn malformed_quoted_chat_is_invalid_argument() {
    let result = QuotedRef::from_proto(quote_from(wire("not a jid")), wire(DM).as_ref());
    match result {
        Err(WamuxError::InvalidArgument(message)) => {
            assert!(message.starts_with("invalid jid 'not a jid'"), "{message}")
        }
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}
