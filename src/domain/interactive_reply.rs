//! Answering a button, a list or a template (issue #28).
//!
//! An offer is answered with a TYPED reply naming the choice by id, never with
//! the button's label as text: a bot matches on the id, and a `conversation`
//! carrying the same words is a different message it does not recognise.
//!
//! Relay-pure, the same way `send_rich::send_contact` is: the edge supplies the
//! ids, the labels and the offer being answered; the core builds the submessage
//! the wire wants and hands it to `Client::send_message`. Nothing is looked up,
//! stored, or guessed — with the two exceptions named below, both of which are
//! the shape of the reply rather than a choice about it.
//!
//! **What here is measured.** 10 replies built by the official WhatsApp client,
//! captured from one live store on 2026-09-23: 5 `buttonsResponseMessage`, 4
//! `listResponseMessage`, 1 `templateButtonReplyMessage`. All of them carry the
//! same field set, which is what makes the two constants below defensible:
//! `buttonsResponseMessage.type = DISPLAY_TEXT` (5 of 5) and
//! `listResponseMessage.listType = SINGLE_SELECT` (4 of 4). They are the only
//! meaningful value of a one-value enum, and no reply was seen without them.
//!
//! **What here is not.** `interactiveResponseMessage` has no capture: the store
//! held 13 interactive offers and not one reply. That shape is built from the
//! proto alone, and it is the one to distrust if a native flow misbehaves.

use whatsapp_rust::buffa::{self, Message as _};
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::{Client, Jid, SendResult};

use crate::domain::outgoing_context::outgoing_context;
use crate::domain::wire_defaults::{nonempty_string, nonzero_i32};
use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

/// Answer an offer. Returns the built message alongside the result so the
/// service can echo it (issue #22), same as every other send the core builds.
pub async fn send_interactive_reply(
    client: &Client,
    to: Jid,
    req: &pb::SendInteractiveReplyRequest,
) -> Result<(SendResult, wa::Message), WamuxError> {
    let message = build_interactive_reply(req)?;
    let result = client
        .send_message(to, message.clone())
        .await
        .map_err(client_err)?;
    Ok((result, message))
}

/// Pure construction of the outgoing reply.
pub(crate) fn build_interactive_reply(
    req: &pb::SendInteractiveReplyRequest,
) -> Result<wa::Message, WamuxError> {
    let context = reply_context(req)?;
    let reply = req
        .reply
        .as_ref()
        .ok_or_else(|| WamuxError::InvalidArgument("no reply shape set".to_string()))?;
    Ok(match reply {
        pb::send_interactive_reply_request::Reply::Button(button) => {
            build_button_reply(button, context)
        }
        pb::send_interactive_reply_request::Reply::List(list) => build_list_reply(list, context),
        pb::send_interactive_reply_request::Reply::Template(template) => {
            build_template_reply(template, context)
        }
        pb::send_interactive_reply_request::Reply::NativeFlow(flow) => {
            build_native_flow_reply(flow, context)
        }
    })
}

/// The quote every reply carries, plus the offer's own payload when the edge
/// supplied it.
///
/// The quote is required rather than optional: a reply that names no offer is
/// not a reply, and relaying one would produce a message the bot cannot tie to
/// its question — a failure that looks like the bot ignoring the user.
fn reply_context(
    req: &pb::SendInteractiveReplyRequest,
) -> Result<buffa::MessageField<wa::ContextInfo>, WamuxError> {
    require_quoted_id(req.quote.as_ref())?;
    let mut context = outgoing_context(&[], req.quote.as_ref(), 0)
        .into_option()
        .ok_or_else(|| WamuxError::InvalidArgument("quote built no context".to_string()))?;
    if !req.quoted_message.is_empty() {
        // Decoded, not blindly attached: bytes that are not a `wa::Message` are
        // the caller's mistake and must say so, rather than riding onto the
        // wire as a malformed quote nobody can read back.
        let quoted = wa::Message::decode(&mut req.quoted_message.as_slice()).map_err(|err| {
            WamuxError::InvalidArgument(format!(
                "quoted_message is not a serialized wa.Message ({} bytes): {err}",
                req.quoted_message.len()
            ))
        })?;
        context.quoted_message = buffa::MessageField::some(quoted);
    }
    Ok(buffa::MessageField::some(context))
}

/// The offer's stanza id, without which there is nothing to answer.
fn require_quoted_id(quote: Option<&pb::QuoteContext>) -> Result<(), WamuxError> {
    let has_id = quote
        .and_then(|quote| quote.quoted.as_ref())
        .is_some_and(|key| !key.id.is_empty());
    if has_id {
        return Ok(());
    }
    Err(WamuxError::InvalidArgument(
        "quote.quoted.id must name the offer being answered".to_string(),
    ))
}

/// `type = DISPLAY_TEXT` is set here, not relayed: 5 of 5 captured replies
/// carry it, and it is the only value the enum has beyond UNKNOWN.
fn build_button_reply(
    reply: &pb::ButtonReply,
    context: buffa::MessageField<wa::ContextInfo>,
) -> wa::Message {
    use wa::message::buttons_response_message::{Response, Type};
    wa::Message {
        buttons_response_message: buffa::MessageField::some(wa::message::ButtonsResponseMessage {
            selected_button_id: nonempty_string(&reply.selected_id),
            response: nonempty_string(&reply.display_text).map(Response::SelectedDisplayText),
            context_info: context,
            r#type: Some(Type::DisplayText),
        }),
        ..Default::default()
    }
}

/// `listType = SINGLE_SELECT` is set here, not relayed: 4 of 4 captured replies
/// carry it, and it is the only value the enum has beyond UNKNOWN.
fn build_list_reply(
    reply: &pb::ListReply,
    context: buffa::MessageField<wa::ContextInfo>,
) -> wa::Message {
    use wa::message::list_response_message::{ListType, SingleSelectReply};
    wa::Message {
        list_response_message: buffa::MessageField::some(wa::message::ListResponseMessage {
            title: nonempty_string(&reply.title),
            list_type: Some(ListType::SingleSelect),
            single_select_reply: buffa::MessageField::some(SingleSelectReply {
                selected_row_id: nonempty_string(&reply.selected_row_id),
            }),
            context_info: context,
            description: nonempty_string(&reply.description),
        }),
        ..Default::default()
    }
}

/// `selected_index` relays verbatim, INCLUDING 0: it is the first button's
/// position, not an absent value, and the captured reply carries an explicit 0.
/// This is the one place the core's proto3-zero-means-absent rule would be
/// wrong.
fn build_template_reply(
    reply: &pb::TemplateReply,
    context: buffa::MessageField<wa::ContextInfo>,
) -> wa::Message {
    wa::Message {
        template_button_reply_message: buffa::MessageField::some(
            wa::message::TemplateButtonReplyMessage {
                selected_id: nonempty_string(&reply.selected_id),
                selected_display_text: nonempty_string(&reply.display_text),
                context_info: context,
                selected_index: Some(reply.selected_index),
                ..Default::default()
            },
        ),
        ..Default::default()
    }
}

/// The shape with no capture behind it (see the module note). Everything here
/// relays verbatim; nothing is set that the edge did not ask for.
fn build_native_flow_reply(
    reply: &pb::NativeFlowReply,
    context: buffa::MessageField<wa::ContextInfo>,
) -> wa::Message {
    use wa::message::interactive_response_message::{
        Body, InteractiveResponseMessage as Which, NativeFlowResponseMessage,
    };
    wa::Message {
        interactive_response_message: buffa::MessageField::some(
            wa::message::InteractiveResponseMessage {
                body: buffa::MessageField::some(Body {
                    text: nonempty_string(&reply.body_text),
                    ..Default::default()
                }),
                context_info: context,
                interactive_response_message: Some(Which::NativeFlowResponseMessage(Box::new(
                    NativeFlowResponseMessage {
                        name: nonempty_string(&reply.name),
                        params_json: nonempty_string(&reply.params_json),
                        version: nonzero_i32(reply.version),
                    },
                ))),
            },
        ),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests;
