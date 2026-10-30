//! A poll published to a channel (#220): the library's `Newsletter::send_poll`.
//!
//! A channel has no end-to-end session, so the poll goes out as the channel's
//! plaintext, as WA Web publishes it: v3 for a single answer, v1 for several
//! (`selectable_count` 0), and no message secret, since a channel's votes are
//! not encrypted. The count is relayed as given; anything above 1 is the
//! library's refusal, not a rewrite. Nothing is served for the channel: the
//! mock only records the stanza.

use tonic::Code;
use wacore_binary::{Node, NodeContent};
use wamux::proto::v1 as pb;
use whatsapp_rust::buffa::Message as _;
use whatsapp_rust::waproto::whatsapp as wa;

use crate::common::mock_wire::{attr, sent_message};
use crate::messaging_service::harness::{
    Fixture, assert_only_the_probe_was_sent, fixture, jid, wire_count,
};

const CHANNEL: &str = "120363000000000220@newsletter";

fn channel_poll(f: &Fixture, options: &[&str], selectable_count: u32) -> pb::SendPollRequest {
    pb::SendPollRequest {
        account: f.a(),
        to: jid(CHANNEL),
        name: "Qual cor?".into(),
        options: options.iter().map(|o| o.to_string()).collect(),
        selectable_count,
    }
}

/// Publish `request` and return the RPC's answer with the stanza it sent.
async fn published(f: &mut Fixture, request: pb::SendPollRequest) -> (pb::SendPollResult, Node) {
    let sent = f
        .messages
        .send_poll(request)
        .await
        .expect("the channel poll is published")
        .into_inner();
    let key = sent.key.clone().expect("key");
    assert_eq!(key.chat, jid(CHANNEL));
    let stanza = sent_message(&f.mock, &key.id).await;
    assert_eq!(attr(&stanza, "to").as_deref(), Some(CHANNEL));
    (sent, stanza)
}

/// The `wa::Message` a channel stanza carries in its `<plaintext>`.
fn plaintext_message(stanza: &Node) -> wa::Message {
    assert!(
        stanza.get_optional_child("enc").is_none(),
        "a channel is not end-to-end"
    );
    let plaintext = stanza
        .get_optional_child("plaintext")
        .expect("a channel send carries <plaintext>");
    let Some(NodeContent::Bytes(bytes)) = &plaintext.content else {
        panic!("<plaintext> carries no bytes: {plaintext:?}");
    };
    wa::Message::decode_from_slice(bytes).expect("a wa::Message")
}

fn option_names(poll: &wa::message::PollCreationMessage) -> Vec<Option<String>> {
    poll.options.iter().map(|o| o.option_name.clone()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_poll_with_one_choice_is_a_v3_without_a_secret() {
    let mut f = fixture("channel_poll_with_one_choice_is_a_v3_without_a_secret").await;
    let request = channel_poll(&f, &["azul", "verde"], 1);
    let (sent, stanza) = published(&mut f, request).await;
    assert!(
        sent.message_secret.is_empty(),
        "a channel poll has no secret to hand back: {:?}",
        sent.message_secret
    );
    let message = plaintext_message(&stanza);
    let poll = message
        .poll_creation_message_v3
        .as_option()
        .expect("a single-answer channel poll is v3");
    assert!(message.poll_creation_message.as_option().is_none());
    assert_eq!(poll.name.as_deref(), Some("Qual cor?"));
    assert_eq!(
        option_names(poll),
        vec![Some("azul".into()), Some("verde".into())]
    );
    assert_eq!(poll.selectable_options_count, Some(1));
    let secret = message
        .message_context_info
        .as_option()
        .and_then(|context| context.message_secret.clone());
    assert_eq!(secret, None, "no secret travels with a channel poll");
    let meta = stanza
        .get_optional_child("meta")
        .expect("a poll creation carries <meta>");
    assert_eq!(attr(meta, "polltype").as_deref(), Some("creation"));
    assert_eq!(attr(meta, "contenttype").as_deref(), Some("text"));
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_poll_with_zero_is_a_v1_for_several_choices() {
    let mut f = fixture("channel_poll_with_zero_is_a_v1_for_several_choices").await;
    let request = channel_poll(&f, &["azul", "verde", "roxo"], 0);
    let (sent, stanza) = published(&mut f, request).await;
    assert!(sent.message_secret.is_empty());
    let message = plaintext_message(&stanza);
    assert!(message.poll_creation_message_v3.as_option().is_none());
    let poll = message
        .poll_creation_message
        .as_option()
        .expect("a several-answer channel poll is v1");
    assert_eq!(
        option_names(poll),
        vec![
            Some("azul".into()),
            Some("verde".into()),
            Some("roxo".into())
        ]
    );
    assert_eq!(poll.selectable_options_count, Some(0));
    f.cleanup().await;
}

/// The library's refusals reach the edge as InvalidArgument with its own
/// words (#101), and nothing is published.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_poll_refuses_what_the_library_refuses() {
    let mut f = fixture("channel_poll_refuses_what_the_library_refuses").await;
    let before = wire_count(&f);
    let thirteen: Vec<String> = (0..13).map(|n| format!("opcao {n}")).collect();
    let thirteen: Vec<&str> = thirteen.iter().map(String::as_str).collect();
    let blank_name = pb::SendPollRequest {
        name: "  ".into(),
        ..channel_poll(&f, &["azul", "verde"], 1)
    };
    let cases = [
        (
            channel_poll(&f, &["azul"], 1),
            "invalid poll: poll must have at least 2 options",
        ),
        (
            channel_poll(&f, &thirteen, 1),
            "invalid poll: polls can have a maximum of 12 options",
        ),
        (
            channel_poll(&f, &["azul", "verde"], 2),
            "invalid poll: channel polls require selectable_count 0 (multiple answers) or 1 \
             (single answer)",
        ),
        (
            channel_poll(&f, &["azul", "azul"], 1),
            "invalid poll: duplicate option name: azul",
        ),
        (
            blank_name,
            "invalid poll: channel poll questions and options cannot be blank",
        ),
        (
            channel_poll(&f, &["azul", " "], 1),
            "invalid poll: channel poll questions and options cannot be blank",
        ),
    ];
    for (request, expected) in cases {
        let status = f.messages.send_poll(request).await.expect_err(expected);
        assert_eq!(
            status.code(),
            Code::InvalidArgument,
            "{expected}: {status:?}"
        );
        assert_eq!(status.message(), expected);
    }
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}
