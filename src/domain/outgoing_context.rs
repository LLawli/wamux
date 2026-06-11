//! Outgoing `wa::ContextInfo` shared by the text and media send paths:
//! mentions, quote, and ephemeral expiration compose identically for every
//! message kind, so both builders call this instead of growing private copies
//! (code-review 2026-06-11: the media path silently dropped quote/mentions).

use whatsapp_rust::waproto::whatsapp as wa;

use crate::domain::wire_defaults::{nonempty_string, nonzero_u32};
use crate::proto::v1 as pb;

/// Build the outgoing ContextInfo, or `None` when every input is the proto3
/// default: a present-but-empty ContextInfo is a wire shape regular clients
/// never produce, so "nothing to relay" must stay the absent field.
pub(crate) fn outgoing_context(
    mentions: &[pb::Mention],
    quote: Option<&pb::QuoteContext>,
    ephemeral_seconds: u32,
) -> Option<Box<wa::ContextInfo>> {
    let mut context = wa::ContextInfo::default();
    if !mentions.is_empty() {
        context.mentioned_jid = mentions.iter().map(|m| m.jid.clone()).collect();
    }
    copy_quote(&mut context, quote);
    context.expiration = nonzero_u32(ephemeral_seconds);
    (context != wa::ContextInfo::default()).then(|| Box::new(context))
}

/// Quote relay: stanza_id + participant from the quoted key, the explicit
/// `QuoteContext.participant` taking precedence. Proto3 empty strings (the
/// normal DM-quote case has no participant) relay as ABSENT waproto fields,
/// never `Some("")` — same rule `wire_defaults` pins everywhere else.
fn copy_quote(context: &mut wa::ContextInfo, quote: Option<&pb::QuoteContext>) {
    let Some(q) = quote else { return };
    let Some(key) = &q.quoted else { return };
    context.stanza_id = nonempty_string(&key.id);
    context.participant =
        nonempty_string(&q.participant).or_else(|| nonempty_string(&key.participant));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quoted_key(participant: &str) -> pb::MessageKey {
        pb::MessageKey {
            remote_jid: "5511999999999@s.whatsapp.net".to_string(),
            id: "QUOTED-1".to_string(),
            from_me: false,
            participant: participant.to_string(),
        }
    }

    #[test]
    fn all_default_inputs_yield_no_context() {
        assert_eq!(outgoing_context(&[], None, 0), None);
    }

    // Quote with no quoted key carries nothing: still no context.
    #[test]
    fn quote_without_quoted_key_yields_no_context() {
        let quote = pb::QuoteContext {
            quoted: None,
            participant: String::new(),
        };
        assert_eq!(outgoing_context(&[], Some(&quote), 0), None);
    }

    // Regression (code-review 2026-06-11): a DM quote has empty participants
    // on both the QuoteContext and the quoted key; that must relay as the
    // absent field, never Some("") — an empty JID on the wire.
    #[test]
    fn dm_quote_with_empty_participants_relays_participant_absent() {
        let quote = pb::QuoteContext {
            quoted: Some(quoted_key("")),
            participant: String::new(),
        };
        let context = outgoing_context(&[], Some(&quote), 0).expect("quote must build a context");
        assert_eq!(context.stanza_id.as_deref(), Some("QUOTED-1"));
        assert_eq!(context.participant, None);
    }

    #[test]
    fn quote_participant_overrides_quoted_key_participant() {
        let quote = pb::QuoteContext {
            quoted: Some(quoted_key("5511777777777@s.whatsapp.net")),
            participant: "5511888888888@s.whatsapp.net".to_string(),
        };
        let context = outgoing_context(&[], Some(&quote), 0).expect("quote must build a context");
        assert_eq!(
            context.participant.as_deref(),
            Some("5511888888888@s.whatsapp.net")
        );
    }

    #[test]
    fn mentions_quote_and_ephemeral_compose() {
        let mentions = [pb::Mention {
            jid: "5511888888888@s.whatsapp.net".to_string(),
        }];
        let quote = pb::QuoteContext {
            quoted: Some(quoted_key("5511777777777@s.whatsapp.net")),
            participant: String::new(),
        };
        let context =
            outgoing_context(&mentions, Some(&quote), 90).expect("inputs must build a context");
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
}
