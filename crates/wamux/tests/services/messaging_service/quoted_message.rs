//! A reply carries the quoted message's own payload when the edge gives it
//! (#230): `quoted_message` on SendText, SendMedia and SendContact goes out as
//! `ContextInfo.quotedMessage`, so a receiver that does not hold the original
//! still shows what was quoted. Without it the quote is the key alone. Proved
//! on what the peer opened.
//!
//! Bytes with no quote to belong to, and bytes that are not a `wa.Message`, are
//! the caller's mistake: InvalidArgument, and nothing reaches the wire, not
//! even the media upload.

use tonic::Code;
use wamux::proto::v1 as pb;
use whatsapp_rust::buffa::Message as _;
use whatsapp_rust::waproto::whatsapp as wa;

use crate::common::mock_wire::sent_message;
use crate::messaging_service::harness::{
    Fixture, assert_only_the_probe_was_sent, fixture, jid, key, sender, text, wire_count,
};
use crate::messaging_service::media::{chunk, head, header, plaintext, send_media};

const QUOTED_ID: &str = "3EB0QUOTED230";

/// Field 1, length-delimited, claiming 60 bytes that are not there.
const NOT_A_WA_MESSAGE: [u8; 4] = [0x0a, 0x3c, 0x01, 0x02];

/// The message being quoted, as the edge stored it.
fn quoted_original() -> wa::Message {
    let mut message = wa::Message::default();
    message.conversation = Some("a mensagem citada".to_string());
    message
}

fn quoting(f: &Fixture) -> Option<pb::QuoteContext> {
    Some(pb::QuoteContext {
        quoted: Some(key(f.peer.pn(), QUOTED_ID)),
        participant: None,
    })
}

fn contact(f: &Fixture) -> pb::SendContactRequest {
    pb::SendContactRequest {
        account: f.a(),
        to: jid(f.peer.pn()),
        display_name: "Maria".into(),
        vcard: "BEGIN:VCARD\nVERSION:3.0\nFN:Maria\nEND:VCARD".into(),
        ..Default::default()
    }
}

async fn opened(f: &mut Fixture, sent: pb::SendResult) -> wa::Message {
    let stanza = sent_message(&f.mock, &sent.key.expect("key").id).await;
    f.peer.open_dm(&stanza, &sender()).await.expect("open")
}

async fn text_context(f: &mut Fixture, quoted_message: Vec<u8>) -> wa::ContextInfo {
    let request = pb::SendTextRequest {
        quote: quoting(f),
        quoted_message,
        ..text(f, f.peer.pn(), "respondendo")
    };
    let sent = f
        .messages
        .send_text(request)
        .await
        .expect("send")
        .into_inner();
    let message = opened(f, sent).await;
    let extended = message.extended_text_message.as_option().expect("extended");
    extended.context_info.as_option().expect("context").clone()
}

async fn media_context(f: &mut Fixture, quoted_message: Vec<u8>) -> wa::ContextInfo {
    let with_quote = pb::SendMediaHeader {
        quote: quoting(f),
        quoted_message,
        ..header(f, pb::MediaType::Image)
    };
    let plain = plaintext(10_000);
    let sent = send_media(f, vec![head(with_quote), chunk(&plain)])
        .await
        .expect("send media");
    let message = opened(f, sent).await;
    let image = message.image_message.as_option().expect("imageMessage");
    image.context_info.as_option().expect("context").clone()
}

async fn contact_context(f: &mut Fixture, quoted_message: Vec<u8>) -> wa::ContextInfo {
    let request = pb::SendContactRequest {
        quote: quoting(f),
        quoted_message,
        ..contact(f)
    };
    let sent = f
        .messages
        .send_contact(request)
        .await
        .expect("send")
        .into_inner();
    let message = opened(f, sent).await;
    let card = message.contact_message.as_option().expect("contactMessage");
    card.context_info.as_option().expect("context").clone()
}

fn assert_carries_the_original(context: &wa::ContextInfo) {
    assert_eq!(context.stanza_id.as_deref(), Some(QUOTED_ID));
    let quoted = context
        .quoted_message
        .as_option()
        .expect("the quoted message rides in the context");
    assert_eq!(quoted, &quoted_original());
}

fn assert_carries_the_key_alone(context: &wa::ContextInfo) {
    assert_eq!(context.stanza_id.as_deref(), Some(QUOTED_ID));
    assert!(
        context.quoted_message.is_unset(),
        "no payload given, none invented"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn text_reply_carries_the_quoted_message() {
    let mut f = fixture("text_reply_carries_the_quoted_message").await;
    let context = text_context(&mut f, quoted_original().encode_to_vec()).await;
    assert_carries_the_original(&context);
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn text_reply_without_quoted_message_carries_none() {
    let mut f = fixture("text_reply_without_quoted_message_carries_none").await;
    let context = text_context(&mut f, Vec::new()).await;
    assert_carries_the_key_alone(&context);
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn media_reply_carries_the_quoted_message() {
    let mut f = fixture("media_reply_carries_the_quoted_message").await;
    let context = media_context(&mut f, quoted_original().encode_to_vec()).await;
    assert_carries_the_original(&context);
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn media_reply_without_quoted_message_carries_none() {
    let mut f = fixture("media_reply_without_quoted_message_carries_none").await;
    let context = media_context(&mut f, Vec::new()).await;
    assert_carries_the_key_alone(&context);
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn contact_reply_carries_the_quoted_message() {
    let mut f = fixture("contact_reply_carries_the_quoted_message").await;
    let context = contact_context(&mut f, quoted_original().encode_to_vec()).await;
    assert_carries_the_original(&context);
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn contact_reply_without_quoted_message_carries_none() {
    let mut f = fixture("contact_reply_without_quoted_message_carries_none").await;
    let context = contact_context(&mut f, Vec::new()).await;
    assert_carries_the_key_alone(&context);
    f.cleanup().await;
}

/// Each send with `quote` and `quoted_message` as given; the outcome only.
async fn send_each(
    f: &mut Fixture,
    quote: Option<pb::QuoteContext>,
    quoted_message: &[u8],
) -> Vec<(&'static str, Result<(), tonic::Status>)> {
    let text_request = pb::SendTextRequest {
        quote: quote.clone(),
        quoted_message: quoted_message.to_vec(),
        ..text(f, f.peer.pn(), "respondendo")
    };
    let text_result = f.messages.send_text(text_request).await.map(drop);
    let media_header = pb::SendMediaHeader {
        quote: quote.clone(),
        quoted_message: quoted_message.to_vec(),
        ..header(f, pb::MediaType::Image)
    };
    let plain = plaintext(10_000);
    let media_result = send_media(f, vec![head(media_header), chunk(&plain)])
        .await
        .map(drop);
    let contact_request = pb::SendContactRequest {
        quote,
        quoted_message: quoted_message.to_vec(),
        ..contact(f)
    };
    let contact_result = f.messages.send_contact(contact_request).await.map(drop);
    vec![
        ("SendText", text_result),
        ("SendMedia", media_result),
        ("SendContact", contact_result),
    ]
}

fn assert_invalid_argument(results: Vec<(&str, Result<(), tonic::Status>)>, says: &str) {
    for (rpc, result) in results {
        let status = result.expect_err(rpc);
        assert_eq!(status.code(), Code::InvalidArgument, "{rpc}: {status:?}");
        assert!(
            status.message().contains(says),
            "{rpc}: expected the message to say {says:?}, got {:?}",
            status.message()
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quoted_message_without_a_quote_is_invalid_argument() {
    let mut f = fixture("quoted_message_without_a_quote_is_invalid_argument").await;
    let before = wire_count(&f);
    let payload = quoted_original().encode_to_vec();
    let results = send_each(&mut f, None, &payload).await;
    assert_invalid_argument(results, "quoted_message without a quote");
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_quoted_message_that_is_not_a_wa_message_is_invalid_argument() {
    let mut f = fixture("a_quoted_message_that_is_not_a_wa_message_is_invalid_argument").await;
    let before = wire_count(&f);
    let quote = quoting(&f);
    let results = send_each(&mut f, quote, &NOT_A_WA_MESSAGE).await;
    assert_invalid_argument(results, "quoted_message is not a serialized wa.Message");
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}
