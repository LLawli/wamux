//! Rich-content sends that build a `wa::Message` and relay it: contact cards
//! (vCard) and polls. Mirrors `messaging::send_reaction` — content fields relay
//! verbatim; the edge supplies the vCard / poll text, the core encodes nothing.

use wamux_types::{ContactCard, Jid, NewPoll, OutgoingContext};
use whatsapp_rust::buffa;
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
    let message = build_contact_message(card);
    client
        .send_message(to.into_lib(), message)
        .await
        .map_err(client_err)
}

/// Pure construction of the outgoing contact `wa::Message`. Empty display_name
/// or vcard relay as ABSENT waproto fields (proto3 default == "not set"), same
/// rule as everywhere else. The optional quote rides on the shared context
/// builder (no mentions on a contact card, so the context carries none).
pub(crate) fn build_contact_message(card: &ContactCard) -> wa::Message {
    let context = OutgoingContext {
        quote: card.quote.clone(),
        ..Default::default()
    };
    wa::Message {
        contact_message: buffa::MessageField::some(wa::message::ContactMessage {
            display_name: nonempty_string(&card.display_name),
            vcard: nonempty_string(&card.vcard),
            context_info: outgoing_context(&context),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Create a poll. The lib validates (2..=12 options, selectable_count
/// 1..=len, no duplicate names) and returns the `SendResult` plus the
/// `message_secret` the edge needs to decrypt incoming votes.
pub async fn send_poll(
    client: &Client,
    to: Jid,
    poll: &NewPoll,
) -> Result<(SendResult, Vec<u8>), WamuxError> {
    client
        .polls()
        .create(
            to.as_lib(),
            &poll.name,
            &poll.options,
            poll.selectable_count,
        )
        .await
        .map_err(client_err)
}

#[cfg(test)]
#[path = "send_rich_tests.rs"]
mod tests;
