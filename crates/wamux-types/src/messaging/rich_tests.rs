//! `ContactCard`, `InteractiveReply`, `ReplyChoice` (#28, #115).

use wamux_proto::v1 as pb;
use wamux_proto::v1::send_interactive_reply_request::Reply;

use crate::{ContactCard, InteractiveReply, Jid, ReplyChoice, WamuxError};

const OFFER_ID: &str = "FA3A44FC17B70123E1";
const MERCHANT: &str = "218562899759170@lid";

fn invalid_argument<T: std::fmt::Debug>(result: Result<T, WamuxError>) -> String {
    match result {
        Err(WamuxError::InvalidArgument(message)) => message,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

fn quote(id: &str, participant: &str) -> pb::QuoteContext {
    pb::QuoteContext {
        quoted: Some(pb::MessageKey {
            chat: Some(pb::Jid {
                value: "5511999999999@s.whatsapp.net".to_string(),
            }),
            id: id.to_string(),
            from_me: false,
            participant: Some(pb::Jid {
                value: participant.to_string(),
            }),
        }),
        participant: None,
    }
}

fn reply_request(
    quote: Option<pb::QuoteContext>,
    reply: Option<Reply>,
) -> pb::SendInteractiveReplyRequest {
    pb::SendInteractiveReplyRequest {
        account: None,
        to: None,
        quote,
        quoted_message: vec![0x0a, 0x01],
        reply,
    }
}

fn button() -> Option<Reply> {
    Some(Reply::Button(pb::ButtonReply {
        selected_id: "op_encerrar".to_string(),
        display_text: "Encerrar".to_string(),
    }))
}

#[test]
fn a_contact_card_carries_its_quote() {
    let card = ContactCard::try_from(pb::SendContactRequest {
        account: None,
        to: None,
        display_name: "Ana".to_string(),
        vcard: "BEGIN:VCARD\nEND:VCARD".to_string(),
        quote: Some(quote("QUOTED-1", "5511777777777@s.whatsapp.net")),
    })
    .unwrap();
    assert_eq!(card.display_name, "Ana");
    assert_eq!(card.vcard, "BEGIN:VCARD\nEND:VCARD");
    let quoted = card.quote.expect("the quote rides along");
    assert_eq!(quoted.id.as_str(), "QUOTED-1");
    assert_eq!(
        quoted.participant,
        Some(Jid::parse("5511777777777@s.whatsapp.net").unwrap())
    );
}

#[test]
fn a_contact_card_without_a_quote_has_none() {
    let card = ContactCard::try_from(pb::SendContactRequest::default()).unwrap();
    assert_eq!(card.quote, None);
    assert_eq!(card.display_name, "");
}

#[test]
fn a_contact_card_refuses_a_quote_with_an_empty_id() {
    let result = ContactCard::try_from(pb::SendContactRequest {
        quote: Some(quote("", "")),
        ..Default::default()
    });
    assert_eq!(
        invalid_argument(result),
        "empty quote.quoted.id; expected the id of the quoted message"
    );
}

#[test]
fn every_reply_shape_converts() {
    let cases = [
        (
            button(),
            ReplyChoice::Button {
                selected_id: "op_encerrar".to_string(),
                display_text: "Encerrar".to_string(),
            },
        ),
        (
            Some(Reply::List(pb::ListReply {
                selected_row_id: "row_1".to_string(),
                title: "Certificados".to_string(),
                description: "Conheça".to_string(),
            })),
            ReplyChoice::List {
                selected_row_id: "row_1".to_string(),
                title: "Certificados".to_string(),
                description: "Conheça".to_string(),
            },
        ),
        (
            Some(Reply::Template(pb::TemplateReply {
                selected_id: "quero".to_string(),
                display_text: "Quero!".to_string(),
                selected_index: 0,
            })),
            ReplyChoice::Template {
                selected_id: "quero".to_string(),
                display_text: "Quero!".to_string(),
                selected_index: 0,
            },
        ),
        (
            Some(Reply::NativeFlow(pb::NativeFlowReply {
                name: "galaxy_message".to_string(),
                params_json: "{}".to_string(),
                body_text: "Enviado".to_string(),
                version: 3,
            })),
            ReplyChoice::NativeFlow {
                name: "galaxy_message".to_string(),
                params_json: "{}".to_string(),
                body_text: "Enviado".to_string(),
                version: 3,
            },
        ),
    ];
    for (reply, expected) in cases {
        let converted =
            InteractiveReply::try_from(reply_request(Some(quote(OFFER_ID, MERCHANT)), reply))
                .unwrap();
        assert_eq!(converted.choice, expected);
        assert_eq!(converted.quote.id.as_str(), OFFER_ID);
        assert_eq!(
            converted.quote.participant,
            Some(Jid::parse(MERCHANT).unwrap())
        );
        assert_eq!(converted.quoted_message, vec![0x0a, 0x01]);
    }
}

// A reply that names no offer is not a reply. The three ways to name none
// answer with the message the domain used, not the generic quote one.
#[test]
fn an_unquoted_or_idless_reply_keeps_its_message() {
    let no_key = pb::QuoteContext {
        quoted: None,
        participant: None,
    };
    for (case, quote) in [
        ("no quote", None),
        ("no quoted key", Some(no_key)),
        ("an empty id", Some(quote("", MERCHANT))),
    ] {
        assert_eq!(
            invalid_argument(InteractiveReply::try_from(reply_request(quote, button()))),
            "quote.quoted.id must name the offer being answered",
            "{case}"
        );
    }
}

#[test]
fn a_reply_with_a_malformed_participant_is_refused() {
    let result =
        InteractiveReply::try_from(reply_request(Some(quote(OFFER_ID, "not a jid")), button()));
    let message = invalid_argument(result);
    assert!(message.starts_with("invalid jid 'not a jid'"), "{message}");
}

#[test]
fn a_reply_without_a_shape_is_refused() {
    let result = InteractiveReply::try_from(reply_request(Some(quote(OFFER_ID, MERCHANT)), None));
    assert_eq!(invalid_argument(result), "no reply shape set");
}

// The offer's id is checked before the shape.
#[test]
fn a_reply_checks_the_offer_before_the_shape() {
    assert_eq!(
        invalid_argument(InteractiveReply::try_from(reply_request(None, None))),
        "quote.quoted.id must name the offer being answered"
    );
}
