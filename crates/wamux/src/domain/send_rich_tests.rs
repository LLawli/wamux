//! `send_rich` (#115): the tests that lived inline, moved so the loop can seal
//! them. Same assertions; the input is the domain's `ContactCard`.

use wamux_types::{ContactCard, Jid, MessageId, QuotedRef};

use super::build_contact_message;

fn card(display_name: &str, vcard: &str) -> ContactCard {
    ContactCard {
        display_name: display_name.to_string(),
        vcard: vcard.to_string(),
        quote: None,
    }
}

#[test]
fn contact_relays_display_name_and_vcard_verbatim() {
    let vcard = "BEGIN:VCARD\nVERSION:3.0\nFN:Ana\nEND:VCARD";
    let message = build_contact_message(&card("Ana", vcard));
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
    let message = build_contact_message(&card("", ""));
    let contact = message
        .contact_message
        .expect("contact_message must be set");
    assert_eq!(contact.display_name, None);
    assert_eq!(contact.vcard, None);
}

#[test]
fn contact_quote_rides_on_context_info() {
    let with_quote = ContactCard {
        quote: Some(QuotedRef {
            id: MessageId::new("QUOTED-1").unwrap(),
            participant: Some(Jid::parse("5511777777777@s.whatsapp.net").unwrap()),
        }),
        ..card("Ana", "BEGIN:VCARD\nEND:VCARD")
    };
    let contact = build_contact_message(&with_quote)
        .contact_message
        .expect("contact_message must be set");
    let context = contact.context_info.expect("quote must build a context");
    assert_eq!(context.stanza_id.as_deref(), Some("QUOTED-1"));
    assert_eq!(
        context.participant.as_deref(),
        Some("5511777777777@s.whatsapp.net")
    );
}
