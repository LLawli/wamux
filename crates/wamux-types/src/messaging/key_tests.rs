//! `MessageTarget`, `QuotedRef`, `OutgoingContext`, `Jid::parse_optional`
//! (#115): what each accepts, what each refuses and with which message.

use wamux_proto::v1 as pb;

use crate::{Jid, MessageId, MessageTarget, OutgoingContext, QuotedRef, WamuxError};

const PHONE: &str = "5511999999999@s.whatsapp.net";
const AUTHOR: &str = "5511777777777@s.whatsapp.net";
const GROUP: &str = "120363001234567890@g.us";

fn invalid_argument<T: std::fmt::Debug>(result: Result<T, WamuxError>) -> String {
    match result {
        Err(WamuxError::InvalidArgument(message)) => message,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

fn key(chat: &str, id: &str, participant: &str) -> pb::MessageKey {
    pb::MessageKey {
        remote_jid: chat.to_string(),
        id: id.to_string(),
        from_me: false,
        participant: participant.to_string(),
    }
}

fn quote(quoted: Option<pb::MessageKey>, participant: &str) -> Option<pb::QuoteContext> {
    Some(pb::QuoteContext {
        quoted,
        participant: participant.to_string(),
    })
}

fn jid(value: &str) -> Jid {
    Jid::parse(value).unwrap()
}

#[test]
fn a_target_converts_every_field() {
    let target = MessageTarget::try_from(pb::MessageKey {
        from_me: true,
        ..key(GROUP, "MSG-1", AUTHOR)
    })
    .unwrap();
    assert_eq!(target.chat, jid(GROUP));
    assert_eq!(target.id, MessageId::new("MSG-1").unwrap());
    assert!(target.from_me);
    assert_eq!(target.participant, Some(jid(AUTHOR)));
}

// Proto3 has no presence on `participant`: an empty string is the DM case and
// must become None, never a parsed empty JID.
#[test]
fn a_target_with_an_empty_participant_is_a_dm() {
    let target = MessageTarget::try_from(key(PHONE, "MSG-2", "")).unwrap();
    assert_eq!(target.participant, None);
}

#[test]
fn a_target_refuses_an_empty_or_malformed_chat() {
    assert_eq!(
        invalid_argument(MessageTarget::try_from(key("", "MSG", ""))),
        "empty jid"
    );
    let message = invalid_argument(MessageTarget::try_from(key("not a jid", "MSG", "")));
    assert!(message.starts_with("invalid jid 'not a jid'"), "{message}");
}

// A malformed participant is the edge's mistake, surfaced as InvalidArgument
// (the core never silently drops a bad JID into None).
#[test]
fn a_target_refuses_a_malformed_participant() {
    let message = invalid_argument(MessageTarget::try_from(key(GROUP, "MSG", "not a jid")));
    assert!(message.starts_with("invalid jid 'not a jid'"), "{message}");
}

#[test]
fn a_target_refuses_an_empty_id() {
    assert_eq!(
        invalid_argument(MessageTarget::try_from(key(PHONE, "", ""))),
        "empty message id"
    );
}

// The chat is checked before the id, the order the domain read them in.
#[test]
fn a_target_checks_the_chat_before_the_id() {
    assert_eq!(
        invalid_argument(MessageTarget::try_from(key("", "", ""))),
        "empty jid"
    );
}

// Decided 2026-10-05: the library's parse is the one normalization, here as
// for `to`. A `@c.us` chat reaches the wire key as `@s.whatsapp.net`.
#[test]
fn a_legacy_target_chat_parses_as_a_phone_user() {
    let target = MessageTarget::try_from(key("5511999999999@c.us", "MSG", "")).unwrap();
    assert_eq!(target.chat.to_string(), PHONE);
}

#[test]
fn parse_optional_reads_empty_as_absence() {
    assert_eq!(Jid::parse_optional("").unwrap(), None);
    assert_eq!(Jid::parse_optional(PHONE).unwrap(), Some(jid(PHONE)));
    let message = invalid_argument(Jid::parse_optional("not a jid"));
    assert!(message.starts_with("invalid jid 'not a jid'"), "{message}");
}

// No quote, and a quote with no quoted key, carry nothing: no quote at all.
#[test]
fn no_quote_or_no_quoted_key_is_no_quote() {
    assert_eq!(QuotedRef::from_proto(None).unwrap(), None);
    assert_eq!(QuotedRef::from_proto(quote(None, "")).unwrap(), None);
    assert_eq!(QuotedRef::from_proto(quote(None, AUTHOR)).unwrap(), None);
}

#[test]
fn a_quote_participant_overrides_the_quoted_key_participant() {
    let quoted = QuotedRef::from_proto(quote(
        Some(key(GROUP, "QUOTED-1", AUTHOR)),
        "5511888888888@s.whatsapp.net",
    ))
    .unwrap()
    .expect("a quoted key is a quote");
    assert_eq!(quoted.id, MessageId::new("QUOTED-1").unwrap());
    assert_eq!(
        quoted.participant,
        Some(jid("5511888888888@s.whatsapp.net"))
    );
}

#[test]
fn a_quote_falls_back_to_the_key_participant() {
    let quoted = QuotedRef::from_proto(quote(Some(key(GROUP, "QUOTED-1", AUTHOR)), ""))
        .unwrap()
        .expect("a quoted key is a quote");
    assert_eq!(quoted.participant, Some(jid(AUTHOR)));
}

// Regression (code-review 2026-06-11): a DM quote has empty participants on
// both the context and the key; that is the absent participant, never Some("").
#[test]
fn a_dm_quote_has_no_participant() {
    let quoted = QuotedRef::from_proto(quote(Some(key(PHONE, "QUOTED-DM", "")), ""))
        .unwrap()
        .expect("a quoted key is a quote");
    assert_eq!(quoted.participant, None);
}

#[test]
fn a_quote_with_an_empty_id_is_refused() {
    assert_eq!(
        invalid_argument(QuotedRef::from_proto(quote(Some(key(PHONE, "", "")), ""))),
        "empty quote.quoted.id; expected the id of the quoted message"
    );
}

#[test]
fn a_quote_with_a_malformed_participant_is_refused() {
    for (context, key_participant) in [("not a jid", ""), ("", "not a jid")] {
        let message = invalid_argument(QuotedRef::from_proto(quote(
            Some(key(PHONE, "QUOTED", key_participant)),
            context,
        )));
        assert!(message.starts_with("invalid jid 'not a jid'"), "{message}");
    }
}

// The wire quote has no chat field, so the quoted key's chat was never read.
// Reading it now would refuse quotes the core always relayed.
#[test]
fn a_quote_never_reads_the_quoted_chat() {
    for chat in ["", "not a jid"] {
        let quoted = QuotedRef::from_proto(quote(Some(key(chat, "QUOTED", "")), ""))
            .unwrap()
            .expect("a quoted key is a quote");
        assert_eq!(quoted.id.as_str(), "QUOTED");
    }
}

#[test]
fn context_parses_mentions_in_order() {
    let mentions = [AUTHOR, "222000222000222@lid"]
        .map(|jid| pb::Mention {
            jid: jid.to_string(),
        })
        .to_vec();
    let context =
        OutgoingContext::from_proto(mentions, quote(Some(key(GROUP, "Q", "")), ""), 90).unwrap();
    assert_eq!(
        context.mentions,
        vec![jid(AUTHOR), jid("222000222000222@lid")]
    );
    assert_eq!(context.quote.expect("quoted").id.as_str(), "Q");
    assert_eq!(context.ephemeral_seconds, 90);
}

#[test]
fn an_empty_context_is_the_default() {
    assert_eq!(
        OutgoingContext::from_proto(Vec::new(), None, 0).unwrap(),
        OutgoingContext::default()
    );
}

// A mention relayed unparsed before #115; it is a jid, so it parses now.
#[test]
fn a_malformed_mention_is_refused() {
    let mention = |jid: &str| {
        vec![pb::Mention {
            jid: jid.to_string(),
        }]
    };
    assert_eq!(
        invalid_argument(OutgoingContext::from_proto(mention(""), None, 0)),
        "empty jid"
    );
    let message = invalid_argument(OutgoingContext::from_proto(mention("not a jid"), None, 0));
    assert!(message.starts_with("invalid jid 'not a jid'"), "{message}");
}

// Mentions are read before the quote.
#[test]
fn a_context_checks_mentions_before_the_quote() {
    let mentions = vec![pb::Mention { jid: String::new() }];
    let result = OutgoingContext::from_proto(mentions, quote(Some(key(PHONE, "", "")), ""), 0);
    assert_eq!(invalid_argument(result), "empty jid");
}
