//! Rich-content sends that build a `wa::Message` and relay it: contact cards
//! (vCard) and polls. Mirrors `messaging::send_reaction` — content fields relay
//! verbatim; the edge supplies the vCard / poll text, the core encodes nothing.

use wamux_types::{ContactCard, Jid, NewPoll, OutgoingContext};
use whatsapp_rust::buffa;
use whatsapp_rust::wacore_binary::JidExt;
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::{Client, SendResult};

use crate::domain::outgoing_context::outgoing_context;
use crate::domain::wire_defaults::nonempty_string;
use crate::error::{WamuxError, client_err};

/// Send a contact card. The edge supplies the full vCard string; the core does
/// not parse, validate, or synthesize it (relay-pure).
pub async fn send_contact(
    client: &Client,
    to: Jid,
    card: &ContactCard,
) -> Result<SendResult, WamuxError> {
    let message = build_contact_message(card)?;
    client
        .send_message(to.into_lib(), message)
        .await
        .map_err(client_err)
}

/// Pure construction of the outgoing contact `wa::Message`. Empty display_name
/// or vcard relay as ABSENT waproto fields (proto3 default == "not set"), same
/// rule as everywhere else. The optional quote rides on the shared context
/// builder (no mentions on a contact card, so the context carries none).
pub(crate) fn build_contact_message(card: &ContactCard) -> Result<wa::Message, WamuxError> {
    let context = OutgoingContext {
        quote: card.quote.clone(),
        ..Default::default()
    };
    let mut contact = wa::message::ContactMessage::default();
    contact.display_name = nonempty_string(&card.display_name);
    contact.vcard = nonempty_string(&card.vcard);
    contact.context_info = outgoing_context(&context)?;
    let mut message = wa::Message::default();
    message.contact_message = buffa::MessageField::some(contact);
    Ok(message)
}

/// Create a poll. The lib validates (2..=12 options, no duplicate names, a
/// `selectable_count` it accepts) and returns the `SendResult` plus the
/// `message_secret` the edge needs to decrypt incoming votes. A refusal is
/// `PollError::InvalidPoll`, which `client_err` maps to InvalidArgument with
/// the lib's reason (#101); the core pre-validates nothing.
///
/// A channel (newsletter) goes another way (#220): there is no end-to-end
/// session, so the poll is published as plain text and has no secret to
/// return. The secret comes back empty and `selectable_count` is passed
/// through as received; the lib decides which values a channel accepts.
pub async fn send_poll(
    client: &Client,
    to: Jid,
    poll: &NewPoll,
) -> Result<(SendResult, Vec<u8>), WamuxError> {
    if to.as_lib().is_newsletter() {
        return send_channel_poll(client, &to, poll).await;
    }
    let created = client
        .polls()
        .create(
            to.as_lib(),
            &poll.name,
            &poll.options,
            poll.selectable_count,
        )
        .await
        .map_err(client_err)?;
    let (result, _creator, secret) = created.into_parts();
    Ok((result, secret.as_bytes().to_vec()))
}

async fn send_channel_poll(
    client: &Client,
    to: &Jid,
    poll: &NewPoll,
) -> Result<(SendResult, Vec<u8>), WamuxError> {
    let result = client
        .newsletter()
        .send_poll(
            to.as_lib(),
            &poll.name,
            &poll.options,
            poll.selectable_count,
        )
        .await
        .map_err(client_err)?;
    Ok((result, Vec::new()))
}

#[cfg(test)]
#[path = "send_rich_tests.rs"]
mod tests;
