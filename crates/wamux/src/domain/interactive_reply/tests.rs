//! `interactive_reply` (#28, #115). Same assertions as before #115; the input
//! is the domain's `InteractiveReply`. The cases about reading the request (a
//! reply with no quote, an idless quote, no shape) moved with that reading to
//! `wamux-types` (`messaging/rich_tests.rs`), with the same assertions.

use wamux_types::{InteractiveReply, Jid, MessageId, QuotedRef, ReplyChoice};
use whatsapp_rust::buffa::Message as _;
use whatsapp_rust::waproto::whatsapp as wa;

use super::build_interactive_reply;
use crate::error::WamuxError;

const OFFER_ID: &str = "FA3A44FC17B70123E1";
const MERCHANT: &str = "218562899759170@lid";

fn quote() -> QuotedRef {
    QuotedRef {
        id: MessageId::new(OFFER_ID).unwrap(),
        participant: Some(Jid::parse(MERCHANT).unwrap()),
    }
}

fn request(choice: ReplyChoice) -> InteractiveReply {
    InteractiveReply {
        quote: quote(),
        quoted_message: Vec::new(),
        choice,
    }
}

fn button(selected_id: &str, display_text: &str) -> ReplyChoice {
    ReplyChoice::Button {
        selected_id: selected_id.to_string(),
        display_text: display_text.to_string(),
    }
}

/// The captured shape: selectedButtonId, selectedDisplayText, contextInfo and
/// `type = DISPLAY_TEXT`, which 5 of 5 official replies carry.
#[test]
fn a_button_reply_names_the_choice_by_id() {
    use wa::message::buttons_response_message::{Response, Type};
    let message = build_interactive_reply(&request(button("op_encerrar", "Encerrar")))
        .expect("a quoted offer plus a shape is all it needs");
    let reply = message
        .buttons_response_message
        .expect("buttons_response_message must be set");
    assert_eq!(reply.selected_button_id.as_deref(), Some("op_encerrar"));
    assert_eq!(
        reply.response,
        Some(Response::SelectedDisplayText("Encerrar".to_string()))
    );
    assert_eq!(reply.r#type, Some(Type::DisplayText));
    let context = reply.context_info.expect("a reply always quotes its offer");
    assert_eq!(context.stanza_id.as_deref(), Some(OFFER_ID));
    assert_eq!(context.participant.as_deref(), Some(MERCHANT));
}

/// The captured shape: the row's id, its title and description, and
/// `listType = SINGLE_SELECT`, which 4 of 4 official replies carry.
#[test]
fn a_list_reply_carries_the_row_id_title_and_description() {
    use wa::message::list_response_message::ListType;
    let message = build_interactive_reply(&request(ReplyChoice::List {
        selected_row_id: "row_certificados".to_string(),
        title: "Certificados Digitais".to_string(),
        description: "Conheça e adquira".to_string(),
    }))
    .expect("a quoted offer plus a shape is all it needs");
    let reply = message
        .list_response_message
        .expect("list_response_message must be set");
    assert_eq!(reply.title.as_deref(), Some("Certificados Digitais"));
    assert_eq!(reply.description.as_deref(), Some("Conheça e adquira"));
    assert_eq!(reply.list_type, Some(ListType::SingleSelect));
    let selected = reply
        .single_select_reply
        .expect("the row id is the whole point");
    assert_eq!(
        selected.selected_row_id.as_deref(),
        Some("row_certificados")
    );
}

/// The trap this core's own conventions would have walked into: `0` is the
/// FIRST button's position, not an absent value, and the captured template
/// reply carries an explicit 0.
#[test]
fn a_template_reply_relays_index_zero_rather_than_dropping_it() {
    let message = build_interactive_reply(&request(ReplyChoice::Template {
        selected_id: "quero_saber_mais".to_string(),
        display_text: "Quero saber mais!".to_string(),
        selected_index: 0,
    }))
    .expect("a quoted offer plus a shape is all it needs");
    let reply = message
        .template_button_reply_message
        .expect("template_button_reply_message must be set");
    assert_eq!(reply.selected_index, Some(0));
    assert_eq!(reply.selected_id.as_deref(), Some("quero_saber_mais"));
    assert_eq!(
        reply.selected_display_text.as_deref(),
        Some("Quero saber mais!")
    );
}

#[test]
fn a_template_reply_relays_a_nonzero_index_too() {
    let message = build_interactive_reply(&request(ReplyChoice::Template {
        selected_id: String::new(),
        display_text: String::new(),
        selected_index: 2,
    }))
    .expect("a quoted offer plus a shape is all it needs");
    let reply = message
        .template_button_reply_message
        .expect("template_button_reply_message must be set");
    assert_eq!(reply.selected_index, Some(2));
}

/// No capture backs this shape, so the test pins the proto reading and nothing
/// more: everything relays verbatim, version 0 leaves the field absent.
#[test]
fn a_native_flow_reply_relays_its_payload_verbatim() {
    use wa::message::interactive_response_message::InteractiveResponseMessage as Which;
    let message = build_interactive_reply(&request(ReplyChoice::NativeFlow {
        name: "galaxy_message".to_string(),
        params_json: r#"{"screen":"WELCOME"}"#.to_string(),
        body_text: "Enviado".to_string(),
        version: 0,
    }))
    .expect("a quoted offer plus a shape is all it needs");
    let reply = message
        .interactive_response_message
        .expect("interactive_response_message must be set");
    assert_eq!(
        reply.body.as_option().and_then(|body| body.text.as_deref()),
        Some("Enviado")
    );
    let Some(Which::NativeFlowResponseMessage(flow)) = reply.interactive_response_message else {
        panic!("the native flow submessage must be set");
    };
    assert_eq!(flow.name.as_deref(), Some("galaxy_message"));
    assert_eq!(flow.params_json.as_deref(), Some(r#"{"screen":"WELCOME"}"#));
    // 0 means "leave it to the proto default of 1".
    assert_eq!(flow.version, None);
}

/// Every captured reply embeds the whole offer, not just its id. The edge has
/// those bytes (`InboundMessage.raw_message`); the core relays them decoded.
#[test]
fn the_offer_payload_rides_in_the_quote_when_supplied() {
    let offer = wa::Message {
        conversation: Some("Escolha uma opção".to_string()),
        ..Default::default()
    };
    let request = InteractiveReply {
        quoted_message: offer.encode_to_vec(),
        ..request(button("op_1", "Um"))
    };
    let context = build_interactive_reply(&request)
        .expect("a quoted offer plus a shape is all it needs")
        .buttons_response_message
        .expect("buttons_response_message must be set")
        .context_info
        .expect("a reply always quotes its offer");
    let quoted = context
        .quoted_message
        .expect("the offer payload must be attached");
    assert_eq!(quoted.conversation.as_deref(), Some("Escolha uma opção"));
}

/// Absent is absent: without the payload the quote still carries the key, and
/// nothing empty is invented in its place.
#[test]
fn an_absent_offer_payload_leaves_the_quote_at_its_key() {
    let context = build_interactive_reply(&request(button("op_1", "Um")))
        .expect("a quoted offer plus a shape is all it needs")
        .buttons_response_message
        .expect("buttons_response_message must be set")
        .context_info
        .expect("a reply always quotes its offer");
    assert!(context.quoted_message.is_unset());
    assert_eq!(context.stanza_id.as_deref(), Some(OFFER_ID));
}

/// Bytes that are not a `wa::Message` are the caller's mistake, and must say so
/// rather than ride onto the wire as a quote nobody can read back.
#[test]
fn a_malformed_offer_payload_is_an_invalid_argument() {
    let request = InteractiveReply {
        // Field 1, length-delimited, claiming 60 bytes that are not there.
        quoted_message: vec![0x0a, 0x3c, 0x01, 0x02],
        ..request(button("op_1", "Um"))
    };
    let err = build_interactive_reply(&request).expect_err("garbage must be refused");
    assert!(matches!(err, WamuxError::InvalidArgument(_)), "{err}");
}

/// Same wire-defaults rule the rest of the core follows: a proto3 empty string
/// relays as the ABSENT waproto field, never `Some("")`.
#[test]
fn empty_strings_relay_as_absent_fields() {
    let reply = build_interactive_reply(&request(button("", "")))
        .expect("a quoted offer plus a shape is all it needs")
        .buttons_response_message
        .expect("buttons_response_message must be set");
    assert_eq!(reply.selected_button_id, None);
    assert_eq!(reply.response, None);
}
