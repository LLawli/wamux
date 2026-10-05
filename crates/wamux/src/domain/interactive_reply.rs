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

use wamux_types::{InteractiveReply, Jid, OutgoingContext, ReplyChoice};
use whatsapp_rust::buffa::{self, Message as _};
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::{Client, SendResult};

use crate::domain::outgoing_context::outgoing_context;
use crate::domain::wire_defaults::{nonempty_string, nonzero_i32};
use crate::error::{WamuxError, client_err};

/// Answer an offer. The built message comes back on `SendResult::message`
/// (upstream #1406) for the service to echo (issue #22), like every send.
pub async fn send_interactive_reply(
    client: &Client,
    to: Jid,
    reply: &InteractiveReply,
) -> Result<SendResult, WamuxError> {
    let message = build_interactive_reply(reply)?;
    client
        .send_message(to.into_lib(), message)
        .await
        .map_err(client_err)
}

/// Pure construction of the outgoing reply.
pub(crate) fn build_interactive_reply(reply: &InteractiveReply) -> Result<wa::Message, WamuxError> {
    let context = reply_context(reply)?;
    Ok(match &reply.choice {
        ReplyChoice::Button {
            selected_id,
            display_text,
        } => build_button_reply(selected_id, display_text, context),
        ReplyChoice::List {
            selected_row_id,
            title,
            description,
        } => build_list_reply(selected_row_id, title, description, context),
        ReplyChoice::Template {
            selected_id,
            display_text,
            selected_index,
        } => build_template_reply(selected_id, display_text, *selected_index, context),
        ReplyChoice::NativeFlow {
            name,
            params_json,
            body_text,
            version,
        } => build_native_flow_reply(name, params_json, body_text, *version, context),
    })
}

/// The quote every reply carries, plus the offer's own payload when the edge
/// supplied it.
///
/// The quote is required rather than optional: a reply that names no offer is
/// not a reply, and relaying one would produce a message the bot cannot tie to
/// its question — a failure that looks like the bot ignoring the user. That is
/// checked when the request is parsed (`InteractiveReply::try_from`, #115).
fn reply_context(
    reply: &InteractiveReply,
) -> Result<buffa::MessageField<wa::ContextInfo>, WamuxError> {
    let quote_only = OutgoingContext {
        quote: Some(reply.quote.clone()),
        ..Default::default()
    };
    let mut context = outgoing_context(&quote_only)
        .into_option()
        .ok_or_else(|| WamuxError::InvalidArgument("quote built no context".to_string()))?;
    if !reply.quoted_message.is_empty() {
        // Decoded, not blindly attached: bytes that are not a `wa::Message` are
        // the caller's mistake and must say so, rather than riding onto the
        // wire as a malformed quote nobody can read back. The decode stays here
        // because it needs waproto, which wamux-types does not depend on.
        let quoted = wa::Message::decode(&mut reply.quoted_message.as_slice()).map_err(|err| {
            WamuxError::InvalidArgument(format!(
                "quoted_message is not a serialized wa.Message ({} bytes): {err}",
                reply.quoted_message.len()
            ))
        })?;
        context.quoted_message = buffa::MessageField::some(quoted);
    }
    Ok(buffa::MessageField::some(context))
}

/// `type = DISPLAY_TEXT` is set here, not relayed: 5 of 5 captured replies
/// carry it, and it is the only value the enum has beyond UNKNOWN.
fn build_button_reply(
    selected_id: &str,
    display_text: &str,
    context: buffa::MessageField<wa::ContextInfo>,
) -> wa::Message {
    use wa::message::buttons_response_message::{Response, Type};
    wa::Message {
        buttons_response_message: buffa::MessageField::some(wa::message::ButtonsResponseMessage {
            selected_button_id: nonempty_string(selected_id),
            response: nonempty_string(display_text).map(Response::SelectedDisplayText),
            context_info: context,
            r#type: Some(Type::DisplayText),
        }),
        ..Default::default()
    }
}

/// `listType = SINGLE_SELECT` is set here, not relayed: 4 of 4 captured replies
/// carry it, and it is the only value the enum has beyond UNKNOWN.
fn build_list_reply(
    selected_row_id: &str,
    title: &str,
    description: &str,
    context: buffa::MessageField<wa::ContextInfo>,
) -> wa::Message {
    use wa::message::list_response_message::{ListType, SingleSelectReply};
    wa::Message {
        list_response_message: buffa::MessageField::some(wa::message::ListResponseMessage {
            title: nonempty_string(title),
            list_type: Some(ListType::SingleSelect),
            single_select_reply: buffa::MessageField::some(SingleSelectReply {
                selected_row_id: nonempty_string(selected_row_id),
            }),
            context_info: context,
            description: nonempty_string(description),
        }),
        ..Default::default()
    }
}

/// `selected_index` relays verbatim, INCLUDING 0: it is the first button's
/// position, not an absent value, and the captured reply carries an explicit 0.
/// This is the one place the core's proto3-zero-means-absent rule would be
/// wrong.
fn build_template_reply(
    selected_id: &str,
    display_text: &str,
    selected_index: u32,
    context: buffa::MessageField<wa::ContextInfo>,
) -> wa::Message {
    wa::Message {
        template_button_reply_message: buffa::MessageField::some(
            wa::message::TemplateButtonReplyMessage {
                selected_id: nonempty_string(selected_id),
                selected_display_text: nonempty_string(display_text),
                context_info: context,
                selected_index: Some(selected_index),
                ..Default::default()
            },
        ),
        ..Default::default()
    }
}

/// The shape with no capture behind it (see the module note). Everything here
/// relays verbatim; nothing is set that the edge did not ask for.
fn build_native_flow_reply(
    name: &str,
    params_json: &str,
    body_text: &str,
    version: i32,
    context: buffa::MessageField<wa::ContextInfo>,
) -> wa::Message {
    use wa::message::interactive_response_message::{
        Body, InteractiveResponseMessage as Which, NativeFlowResponseMessage,
    };
    wa::Message {
        interactive_response_message: buffa::MessageField::some(
            wa::message::InteractiveResponseMessage {
                body: buffa::MessageField::some(Body {
                    text: nonempty_string(body_text),
                    ..Default::default()
                }),
                context_info: context,
                interactive_response_message: Some(Which::NativeFlowResponseMessage(Box::new(
                    NativeFlowResponseMessage {
                        name: nonempty_string(name),
                        params_json: nonempty_string(params_json),
                        version: nonzero_i32(version),
                    },
                ))),
            },
        ),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests;
