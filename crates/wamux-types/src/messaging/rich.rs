//! A contact card, and the typed answer to a button, list or template (#28, #115).

use wamux_proto::v1 as pb;

use crate::error::WamuxError;
use crate::messaging::key::QuotedRef;

/// A contact card. The vCard is the edge's: the core neither parses nor builds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactCard {
    pub display_name: String,
    pub vcard: String,
    pub quote: Option<QuotedRef>,
}

/// Which offer shape is answered, and with what. Strings relay verbatim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplyChoice {
    Button {
        selected_id: String,
        display_text: String,
    },
    List {
        selected_row_id: String,
        title: String,
        description: String,
    },
    Template {
        selected_id: String,
        display_text: String,
        /// 0 is the first button, not an absent value.
        selected_index: u32,
    },
    NativeFlow {
        name: String,
        params_json: String,
        body_text: String,
        version: i32,
    },
}

/// The answer to an offer. The quote is required: a reply that names no offer
/// is not a reply. `quoted_message` stays raw bytes; the domain decodes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InteractiveReply {
    pub quote: QuotedRef,
    pub quoted_message: Vec<u8>,
    pub choice: ReplyChoice,
}

impl TryFrom<pb::SendContactRequest> for ContactCard {
    type Error = WamuxError;

    fn try_from(request: pb::SendContactRequest) -> Result<Self, WamuxError> {
        Ok(Self {
            quote: QuotedRef::from_proto(request.quote)?,
            display_name: request.display_name,
            vcard: request.vcard,
        })
    }
}

/// The offer's id first: no quote, no quoted key or an empty id is
/// `InvalidArgument("quote.quoted.id must name the offer being answered")`,
/// as the domain answered. Then the quote's participant, then the shape
/// (`"no reply shape set"`).
impl TryFrom<pb::SendInteractiveReplyRequest> for InteractiveReply {
    type Error = WamuxError;

    fn try_from(request: pb::SendInteractiveReplyRequest) -> Result<Self, WamuxError> {
        let quote = offer_quote(request.quote)?;
        let choice = ReplyChoice::try_from(request.reply)?;
        Ok(Self {
            quote,
            quoted_message: request.quoted_message,
            choice,
        })
    }
}

/// No quote, no quoted key and an empty id are all the same mistake: nothing
/// names the offer. `QuotedRef` would call the last one something else, so the
/// id is checked here first, under the reply's own message.
fn offer_quote(quote: Option<pb::QuoteContext>) -> Result<QuotedRef, WamuxError> {
    let names_the_offer = quote
        .as_ref()
        .and_then(|quote| quote.quoted.as_ref())
        .is_some_and(|key| !key.id.is_empty());
    let missing = || {
        WamuxError::InvalidArgument(
            "quote.quoted.id must name the offer being answered".to_string(),
        )
    };
    if !names_the_offer {
        return Err(missing());
    }
    QuotedRef::from_proto(quote)?.ok_or_else(missing)
}

impl TryFrom<Option<pb::send_interactive_reply_request::Reply>> for ReplyChoice {
    type Error = WamuxError;

    fn try_from(
        reply: Option<pb::send_interactive_reply_request::Reply>,
    ) -> Result<Self, WamuxError> {
        use pb::send_interactive_reply_request::Reply;
        let reply =
            reply.ok_or_else(|| WamuxError::InvalidArgument("no reply shape set".to_string()))?;
        Ok(match reply {
            Reply::Button(button) => Self::Button {
                selected_id: button.selected_id,
                display_text: button.display_text,
            },
            Reply::List(list) => Self::List {
                selected_row_id: list.selected_row_id,
                title: list.title,
                description: list.description,
            },
            Reply::Template(template) => Self::Template {
                selected_id: template.selected_id,
                display_text: template.display_text,
                selected_index: template.selected_index,
            },
            Reply::NativeFlow(flow) => Self::NativeFlow {
                name: flow.name,
                params_json: flow.params_json,
                body_text: flow.body_text,
                version: flow.version,
            },
        })
    }
}
