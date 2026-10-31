//! Outgoing `wa::ContextInfo` shared by the text, media, contact and
//! interactive-reply send paths: mentions, quote (with the quoted message's
//! own payload, #230), and ephemeral expiration compose identically for every
//! message kind, so every builder calls this instead of growing private copies
//! that drift apart (a media path that drops the quote or the mentions).

use wamux_types::{OutgoingContext, QuotedRef, WamuxError};
use whatsapp_rust::buffa::{Message as _, MessageField};
use whatsapp_rust::waproto::whatsapp as wa;

use crate::domain::wire_defaults::nonzero_u32;

/// Build the outgoing ContextInfo, or the unset field when every input is the
/// proto3 default: a present-but-empty ContextInfo is a wire shape regular
/// clients never produce, so "nothing to relay" must stay the absent field.
///
/// Returns buffa's `MessageField` (waproto's sub-message slot since 0.7) rather
/// than `Option<Box<_>>`, so every caller can drop it straight into the message
/// literal without a conversion at each site.
pub(crate) fn outgoing_context(
    context: &OutgoingContext,
) -> Result<MessageField<wa::ContextInfo>, WamuxError> {
    let mut info = wa::ContextInfo::default();
    if !context.mentions.is_empty() {
        info.mentioned_jid = context.mentions.iter().map(|jid| jid.to_string()).collect();
    }
    if let Some(quote) = &context.quote {
        copy_quote(&mut info, quote)?;
    }
    info.expiration = nonzero_u32(context.ephemeral_seconds);
    if info == wa::ContextInfo::default() {
        return Ok(MessageField::none());
    }
    Ok(MessageField::some(info))
}

/// Quote relay: the quoted stanza id, its author in a group, and the quoted
/// message's chat when it differs from the reply's (a private reply to a group
/// message, a reply to a status), which WhatsApp carries as `remote_jid`. A
/// same-chat reply leaves it unset, as the official clients do. A DM quote
/// has no participant, which relays as the ABSENT waproto field, never
/// `Some("")`: the rule `wire_defaults` pins everywhere else. Which participant
/// wins (the context's or the key's) was decided when the quote was parsed.
/// A non-empty `quote.message` is decoded into `quoted_message`, so receivers
/// without the original still show the quote; an undecodable one is refused.
fn copy_quote(info: &mut wa::ContextInfo, quote: &QuotedRef) -> Result<(), WamuxError> {
    info.stanza_id = Some(quote.id.to_string());
    info.participant = quote.participant.as_ref().map(ToString::to_string);
    info.remote_jid = quote.remote_chat.as_ref().map(ToString::to_string);
    if !quote.message.is_empty() {
        info.quoted_message = MessageField::some(decode_quoted_message(&quote.message)?);
    }
    Ok(())
}

/// Bytes that are not a `wa::Message` are the caller's mistake and must say so,
/// not ride onto the wire as a malformed quote nobody can read back (#230).
/// Decoded here, not in wamux-types, which does not depend on waproto.
fn decode_quoted_message(bytes: &[u8]) -> Result<wa::Message, WamuxError> {
    wa::Message::decode(&mut &bytes[..]).map_err(|err| {
        WamuxError::InvalidArgument(format!(
            "quoted_message is not a serialized wa.Message ({} bytes): {err}",
            bytes.len()
        ))
    })
}

#[cfg(test)]
#[path = "outgoing_context_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "outgoing_context_remote_tests.rs"]
mod remote_tests;
