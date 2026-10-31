//! #230: the quoted message's own payload rides in the quote. It is raw bytes
//! here (the domain decodes them), it belongs to a quote, and bytes with no
//! quote to belong to are the caller's mistake.

use wamux_proto::v1 as pb;

use crate::{QuotedRef, WamuxError};

const DM: &str = "5511999999999@s.whatsapp.net";

/// A serialized `wa.Message` with `conversation = "oi"`: field 1, 2 bytes.
const QUOTED_PAYLOAD: [u8; 4] = [0x0a, 0x02, b'o', b'i'];

fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

fn quote_of(quoted: Option<pb::MessageKey>) -> Option<pb::QuoteContext> {
    Some(pb::QuoteContext {
        quoted,
        participant: None,
    })
}

fn key() -> pb::MessageKey {
    pb::MessageKey {
        chat: wire(DM),
        id: "QUOTED-230".to_string(),
        from_me: false,
        participant: None,
    }
}

fn invalid_argument(result: Result<Option<QuotedRef>, WamuxError>) -> String {
    match result {
        Err(WamuxError::InvalidArgument(message)) => message,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

#[test]
fn a_quote_takes_the_quoted_message_bytes() {
    let quote = QuotedRef::from_proto(
        quote_of(Some(key())),
        QUOTED_PAYLOAD.to_vec(),
        wire(DM).as_ref(),
    )
    .expect("a quote with its payload")
    .expect("a quoted key is a quote");
    assert_eq!(quote.message, QUOTED_PAYLOAD.to_vec());
    assert_eq!(quote.id.to_string(), "QUOTED-230");
}

#[test]
fn quoted_message_without_a_quote_context_is_invalid_argument() {
    let message = invalid_argument(QuotedRef::from_proto(
        None,
        QUOTED_PAYLOAD.to_vec(),
        wire(DM).as_ref(),
    ));
    assert_eq!(
        message,
        "quoted_message without a quote; set quote.quoted to the message it is"
    );
}

#[test]
fn quoted_message_with_a_keyless_quote_is_invalid_argument() {
    let message = invalid_argument(QuotedRef::from_proto(
        quote_of(None),
        QUOTED_PAYLOAD.to_vec(),
        wire(DM).as_ref(),
    ));
    assert_eq!(
        message,
        "quoted_message without a quote; set quote.quoted to the message it is"
    );
}
