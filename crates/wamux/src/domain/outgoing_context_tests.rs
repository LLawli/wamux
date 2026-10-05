//! `outgoing_context` (#115): the tests that lived inline, moved so the loop
//! can seal them. Same assertions; the inputs are the domain's own types. The
//! cases that were about reading a `pb::QuoteContext` (a quote with no key, a
//! participant overriding the key's) moved with that reading to
//! `wamux-types` (`messaging/key_tests.rs`).

use wamux_types::{Jid, MessageId, OutgoingContext, QuotedRef};

use super::outgoing_context;

fn jid(value: &str) -> Jid {
    Jid::parse(value).unwrap()
}

fn quote(participant: Option<&str>) -> Option<QuotedRef> {
    Some(QuotedRef {
        id: MessageId::new("QUOTED-1").unwrap(),
        participant: participant.map(jid),
    })
}

#[test]
fn all_default_inputs_yield_no_context() {
    assert!(outgoing_context(&OutgoingContext::default()).is_unset());
}

// Regression (code-review 2026-06-11): a DM quote has no participant; that
// must relay as the absent field, never Some("") — an empty JID on the wire.
#[test]
fn dm_quote_without_participant_relays_participant_absent() {
    let context = outgoing_context(&OutgoingContext {
        quote: quote(None),
        ..Default::default()
    })
    .expect("quote must build a context");
    assert_eq!(context.stanza_id.as_deref(), Some("QUOTED-1"));
    assert_eq!(context.participant, None);
}

#[test]
fn a_quote_relays_its_participant() {
    let context = outgoing_context(&OutgoingContext {
        quote: quote(Some("5511888888888@s.whatsapp.net")),
        ..Default::default()
    })
    .expect("quote must build a context");
    assert_eq!(
        context.participant.as_deref(),
        Some("5511888888888@s.whatsapp.net")
    );
}

#[test]
fn mentions_quote_and_ephemeral_compose() {
    let context = outgoing_context(&OutgoingContext {
        mentions: vec![jid("5511888888888@s.whatsapp.net")],
        quote: quote(Some("5511777777777@s.whatsapp.net")),
        ephemeral_seconds: 90,
    })
    .expect("inputs must build a context");
    assert_eq!(
        context.mentioned_jid,
        vec!["5511888888888@s.whatsapp.net".to_string()]
    );
    assert_eq!(context.stanza_id.as_deref(), Some("QUOTED-1"));
    assert_eq!(
        context.participant.as_deref(),
        Some("5511777777777@s.whatsapp.net")
    );
    assert_eq!(context.expiration, Some(90));
}

// Mentions relay in the order given, as the library printed each jid.
#[test]
fn mentions_relay_in_order() {
    let context = outgoing_context(&OutgoingContext {
        mentions: vec![
            jid("5511888888888@s.whatsapp.net"),
            jid("222000222000222@lid"),
        ],
        ..Default::default()
    })
    .expect("mentions build a context");
    assert_eq!(
        context.mentioned_jid,
        vec![
            "5511888888888@s.whatsapp.net".to_string(),
            "222000222000222@lid".to_string()
        ]
    );
}
