//! The message a request acts on, the message it quotes, and the context an
//! outgoing message carries (#115).

use wamux_proto::v1 as pb;

use crate::error::WamuxError;
use crate::jid::Jid;
use crate::message_id::MessageId;

/// The message a reaction, an edit, a delete or a star acts on.
///
/// `participant` is the author inside a group; `None` is a DM, where proto3's
/// empty string is the wire's way of saying so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageTarget {
    pub chat: Jid,
    pub id: MessageId,
    pub from_me: bool,
    pub participant: Option<Jid>,
}

/// The message an outgoing one quotes: its stanza id, and who wrote it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuotedRef {
    pub id: MessageId,
    pub participant: Option<Jid>,
}

/// What every outgoing text, media and contact card carries besides its body:
/// mentions, a quote, the disappearing timer. All defaults is "nothing".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutgoingContext {
    pub mentions: Vec<Jid>,
    pub quote: Option<QuotedRef>,
    /// 0 is "not ephemeral": the core invents no duration.
    pub ephemeral_seconds: u32,
}

/// Checked in the order the domain read them: chat, participant, id.
/// `InvalidArgument("missing jid")` / `("invalid jid '<v>': ..")` for the chat
/// (unset or empty is missing), `("invalid jid '<v>': ..")` for a malformed
/// participant (unset or empty is a DM, #120), `("empty message id")` for the id.
impl TryFrom<pb::MessageKey> for MessageTarget {
    type Error = WamuxError;

    fn try_from(key: pb::MessageKey) -> Result<Self, WamuxError> {
        Ok(Self {
            chat: Jid::from_required_wire(key.chat)?,
            participant: Jid::from_optional_wire(key.participant)?,
            id: MessageId::new(key.id)?,
            from_me: key.from_me,
        })
    }
}

impl QuotedRef {
    /// A request's optional quote. No context, or a context with no `quoted`
    /// key, is no quote (as it always was). A key with an empty id is
    /// `InvalidArgument("empty quote.quoted.id; expected the id of the quoted
    /// message")`. The participant is the context's own when it is set and not
    /// empty (#120), else the quoted key's. The quoted key's `chat` is never
    /// read: the wire quote has no field for it.
    pub fn from_proto(quote: Option<pb::QuoteContext>) -> Result<Option<Self>, WamuxError> {
        let Some(quoted_key) = quote.as_ref().and_then(|quote| quote.quoted.as_ref()) else {
            return Ok(None);
        };
        if quoted_key.id.is_empty() {
            return Err(WamuxError::InvalidArgument(
                "empty quote.quoted.id; expected the id of the quoted message".to_string(),
            ));
        }
        // The context's own participant wins; the key's is the fallback, as the
        // wire builder always read them.
        let context_participant = quote.as_ref().and_then(|quote| quote.participant.clone());
        let participant: Option<Jid> = match Jid::from_optional_wire(context_participant)? {
            Some(own) => Some(own),
            None => Jid::from_optional_wire(quoted_key.participant.clone())?,
        };
        Ok(Some(Self {
            id: MessageId::new(quoted_key.id.clone())?,
            participant,
        }))
    }
}

impl OutgoingContext {
    /// Mentions first, in the order given, then the quote.
    pub fn from_proto(
        mentions: Vec<pb::Mention>,
        quote: Option<pb::QuoteContext>,
        ephemeral_seconds: u32,
    ) -> Result<Self, WamuxError> {
        let mentions = mentions
            .iter()
            .map(|mention| Jid::from_required_wire(mention.jid.clone()))
            .collect::<Result<Vec<Jid>, WamuxError>>()?;
        Ok(Self {
            mentions,
            quote: QuotedRef::from_proto(quote)?,
            ephemeral_seconds,
        })
    }
}
