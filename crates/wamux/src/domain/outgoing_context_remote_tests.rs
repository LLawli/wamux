//! #217: an outgoing quote's remote chat becomes the context's `remoteJid`.
//! Whether a quoted chat is "remote" (not the destination) is decided when the
//! request is parsed, in `wamux-types` (`messaging/quote_chat_tests.rs`).

use wamux_types::{Jid, MessageId, OutgoingContext, QuotedRef};

use super::outgoing_context;

const GROUP: &str = "120363041234567217@g.us";
const AUTHOR: &str = "5511888000217@s.whatsapp.net";

fn context_quoting(remote_chat: Option<&str>) -> OutgoingContext {
    OutgoingContext {
        quote: Some(QuotedRef {
            id: MessageId::new("QUOTED-217").unwrap(),
            participant: Some(Jid::parse(AUTHOR).unwrap()),
            remote_chat: remote_chat.map(|chat| Jid::parse(chat).unwrap()),
            message: Vec::new(),
        }),
        ..Default::default()
    }
}

#[test]
fn remote_chat_becomes_remote_jid() {
    let context = outgoing_context(&context_quoting(Some(GROUP)))
        .expect("the context builds")
        .into_option()
        .expect("a quote builds a context");
    assert_eq!(context.remote_jid.as_deref(), Some(GROUP));
    assert_eq!(context.stanza_id.as_deref(), Some("QUOTED-217"));
    assert_eq!(context.participant.as_deref(), Some(AUTHOR));
}

#[test]
fn no_remote_chat_leaves_remote_jid_unset() {
    let context = outgoing_context(&context_quoting(None))
        .expect("the context builds")
        .into_option()
        .expect("a quote builds a context");
    assert_eq!(
        context.remote_jid, None,
        "a reply in the same chat names no other chat"
    );
}
