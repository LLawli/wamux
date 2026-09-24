//! Rich-content sends that build a `wa::Message` and relay it: contact cards
//! (vCard) and polls. Mirrors `messaging::send_reaction` — content fields relay
//! verbatim; the edge supplies the vCard / poll text, the core encodes nothing.

use whatsapp_rust::buffa;
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::{Client, Jid, SendResult};

use crate::domain::outgoing_context::outgoing_context;
use crate::domain::wire_defaults::nonempty_string;
use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

/// Send a contact card. The edge supplies the full vCard string; the core does
/// not parse, validate, or synthesize it (relay-pure).
pub async fn send_contact(
    client: &Client,
    to: Jid,
    req: &pb::SendContactRequest,
) -> Result<SendResult, WamuxError> {
    let message = build_contact_message(req);
    client.send_message(to, message).await.map_err(client_err)
}

/// Pure construction of the outgoing contact `wa::Message`. Empty display_name
/// or vcard relay as ABSENT waproto fields (proto3 default == "not set"), same
/// rule as everywhere else. The optional quote rides on the shared context
/// builder (no mentions on a contact card, so an empty slice is passed).
pub(crate) fn build_contact_message(req: &pb::SendContactRequest) -> wa::Message {
    wa::Message {
        contact_message: buffa::MessageField::some(wa::message::ContactMessage {
            display_name: nonempty_string(&req.display_name),
            vcard: nonempty_string(&req.vcard),
            context_info: outgoing_context(&[], req.quote.as_ref(), 0),
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
    req: &pb::SendPollRequest,
) -> Result<(SendResult, Vec<u8>), WamuxError> {
    client
        .polls()
        .create(&to, &req.name, &req.options, req.selectable_count)
        .await
        .map_err(client_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact_req(display_name: &str, vcard: &str) -> pb::SendContactRequest {
        pb::SendContactRequest {
            display_name: display_name.to_string(),
            vcard: vcard.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn contact_relays_display_name_and_vcard_verbatim() {
        let vcard = "BEGIN:VCARD\nVERSION:3.0\nFN:Ana\nEND:VCARD";
        let message = build_contact_message(&contact_req("Ana", vcard));
        let contact = message
            .contact_message
            .expect("contact_message must be set");
        assert_eq!(contact.display_name.as_deref(), Some("Ana"));
        assert_eq!(contact.vcard.as_deref(), Some(vcard));
        // No quote: the shared context builder yields None, never an empty one.
        assert!(contact.context_info.is_unset());
    }

    // Proto3 defaults (empty display_name / vcard) relay as ABSENT waproto
    // fields, never Some("") — same wire-defaults rule pinned across the core.
    #[test]
    fn contact_empty_fields_map_to_none() {
        let message = build_contact_message(&contact_req("", ""));
        let contact = message
            .contact_message
            .expect("contact_message must be set");
        assert_eq!(contact.display_name, None);
        assert_eq!(contact.vcard, None);
    }

    #[test]
    fn contact_quote_rides_on_context_info() {
        let req = pb::SendContactRequest {
            quote: Some(pb::QuoteContext {
                quoted: Some(pb::MessageKey {
                    remote_jid: "120363001234567890@g.us".to_string(),
                    id: "QUOTED-1".to_string(),
                    from_me: false,
                    participant: "5511777777777@s.whatsapp.net".to_string(),
                }),
                participant: String::new(),
            }),
            ..contact_req("Ana", "BEGIN:VCARD\nEND:VCARD")
        };
        let contact = build_contact_message(&req)
            .contact_message
            .expect("contact_message must be set");
        let context = contact.context_info.expect("quote must build a context");
        assert_eq!(context.stanza_id.as_deref(), Some("QUOTED-1"));
        assert_eq!(
            context.participant.as_deref(),
            Some("5511777777777@s.whatsapp.net")
        );
    }
}
