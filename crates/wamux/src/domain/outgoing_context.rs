//! Outgoing `wa::ContextInfo` shared by the text and media send paths:
//! mentions, quote, and ephemeral expiration compose identically for every
//! message kind, so both builders call this instead of growing private copies
//! (code-review 2026-06-11: the media path silently dropped quote/mentions).

use wamux_types::{OutgoingContext, QuotedRef};
use whatsapp_rust::buffa::MessageField;
use whatsapp_rust::waproto::whatsapp as wa;

use crate::domain::wire_defaults::nonzero_u32;

/// Build the outgoing ContextInfo, or the unset field when every input is the
/// proto3 default: a present-but-empty ContextInfo is a wire shape regular
/// clients never produce, so "nothing to relay" must stay the absent field.
///
/// Returns buffa's `MessageField` (waproto's sub-message slot since 0.7) rather
/// than `Option<Box<_>>`, so every caller can drop it straight into the message
/// literal without a conversion at each site.
pub(crate) fn outgoing_context(context: &OutgoingContext) -> MessageField<wa::ContextInfo> {
    let mut info = wa::ContextInfo::default();
    if !context.mentions.is_empty() {
        info.mentioned_jid = context.mentions.iter().map(|jid| jid.to_string()).collect();
    }
    if let Some(quote) = &context.quote {
        copy_quote(&mut info, quote);
    }
    info.expiration = nonzero_u32(context.ephemeral_seconds);
    if info == wa::ContextInfo::default() {
        return MessageField::none();
    }
    MessageField::some(info)
}

/// Quote relay: the quoted stanza id and, in a group, its author. A DM quote
/// has no participant, which relays as the ABSENT waproto field, never
/// `Some("")`: the rule `wire_defaults` pins everywhere else. Which participant
/// wins (the context's or the key's) was decided when the quote was parsed.
fn copy_quote(info: &mut wa::ContextInfo, quote: &QuotedRef) {
    info.stanza_id = Some(quote.id.to_string());
    info.participant = quote.participant.as_ref().map(ToString::to_string);
}

#[cfg(test)]
#[path = "outgoing_context_tests.rs"]
mod tests;
