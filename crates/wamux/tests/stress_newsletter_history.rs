//! Channel history through the library (issue #56): what `GetNewsletterMessages`
//! relays for the pages the server sends.
//!
//! The mock answers the `newsletter` IQ with a page a test builds, and a real,
//! unmodified `whatsapp-rust` Client reads it through
//! `client.newsletter().get_messages()`, the path production takes. The page
//! is the live-captured shape from #26 plus the edge cases #40, #43, #44 and
//! #51 pinned when the core still parsed the IQ itself; every one of them is
//! asserted by value here, so the swap to the library is held to them.
//!
//! Run with: `cargo test --features stress --test stress_newsletter_history`.
//! Requires the docker Postgres (DATABASE_URL).
#![cfg(feature = "stress")]

use wacore_binary::Node;
use wacore_binary::builder::NodeBuilder;
use wamux::domain::newsletters;
use wamux::proto::v1 as pb;
use wamux::stress::MockWaServer;
use whatsapp_rust::Client;
use whatsapp_rust::buffa::Message as _;
use whatsapp_rust::waproto::whatsapp as wa;

#[allow(dead_code)]
mod common;

const CHANNEL: &str = "120363144038483540@newsletter";
const POLL_OPTION: &str = "azul";

/// A `<message>` row with the attributes every captured row carries.
fn row(server_id: &str, kind: Option<&str>) -> NodeBuilder {
    let mut node = NodeBuilder::new("message")
        .attr("server_id", server_id)
        .attr("id", format!("3AEC{server_id}"))
        .attr("t", "1790001172");
    if let Some(kind) = kind {
        node = node.attr("type", kind);
    }
    node
}

fn text_payload(text: &str) -> Vec<u8> {
    wa::Message {
        conversation: Some(text.to_string()),
        ..Default::default()
    }
    .encode_to_vec()
}

fn plaintext(bytes: Vec<u8>) -> Node {
    NodeBuilder::new("plaintext").bytes(bytes).build()
}

fn vote(count: Option<&str>, hash: Vec<u8>) -> Node {
    let mut node = NodeBuilder::new("vote");
    if let Some(count) = count {
        node = node.attr("count", count);
    }
    node.bytes(hash).build()
}

fn meta_polltype(value: &str) -> Node {
    NodeBuilder::new("meta").attr("polltype", value).build()
}

fn option_hash() -> Vec<u8> {
    wacore::poll::compute_option_hash(POLL_OPTION).to_vec()
}

/// #26's capture, plus revoked and undecodable bodies, an unknown type, an
/// admin edit with its times (#51), and a row nothing can address.
fn captured_rows() -> Vec<Node> {
    vec![
        row("773", Some("media"))
            .children([
                NodeBuilder::new("forwards_count")
                    .attr("count", "8329")
                    .build(),
                NodeBuilder::new("reactions")
                    .children([
                        NodeBuilder::new("reaction")
                            .attr("code", "\u{1F621}")
                            .attr("count", "437")
                            .build(),
                        // No code: a count for an emoji nobody can name.
                        NodeBuilder::new("reaction").attr("count", "9").build(),
                    ])
                    .build(),
                plaintext(text_payload("bom dia")),
            ])
            .build(),
        row("777", Some("poll"))
            .children([
                meta_polltype("creation"),
                NodeBuilder::new("votes")
                    .children([vote(Some("25328"), option_hash())])
                    .build(),
                plaintext(text_payload("qual a sua?")),
            ])
            .build(),
        // Revoked: the server keeps the envelope and empties the body.
        row("780", Some("text"))
            .attr("edit", "8")
            .children([NodeBuilder::new("plaintext").build()])
            .build(),
        row("781", Some("text"))
            .attr("edit", "8")
            .children([plaintext(Vec::new())])
            .build(),
        // Bytes that are not a wa.Message.
        row("782", Some("text"))
            .children([plaintext(vec![0xff, 0xff, 0xff])])
            .build(),
        row("783", Some("hologram"))
            .children([plaintext(text_payload("?"))])
            .build(),
        // #51: an admin edit carries both times on `<meta>`, in two units.
        row("784", Some("text"))
            .attr("edit", "3")
            .attr("is_sender", "true")
            .children([
                NodeBuilder::new("meta")
                    .attr("original_msg_t", "1790001172")
                    .attr("msg_edit_t", "1790004321987")
                    .build(),
                plaintext(text_payload("editado")),
            ])
            .build(),
        // No server_id: nothing can address it, so it is skipped.
        NodeBuilder::new("message")
            .attr("id", "NOSERVERID")
            .attr("type", "text")
            .build(),
    ]
}

fn page(rows: Vec<Node>) -> Node {
    NodeBuilder::new("messages")
        .attr("jid", CHANNEL)
        .children(rows)
        .build()
}

fn history_request(count: u32) -> pb::GetNewsletterMessagesRequest {
    pb::GetNewsletterMessagesRequest {
        account: None,
        jid: CHANNEL.to_string(),
        count,
        before: 0,
    }
}

/// The core's answer to one page.
async fn core_reads(
    client: &Client,
    mock: &MockWaServer,
    rows: Vec<Node>,
) -> Vec<pb::NewsletterMessage> {
    mock.answer_newsletter_iq_with(page(rows));
    newsletters::get_messages(client, &history_request(20))
        .await
        .expect("core get_messages")
        .messages
}

fn inbound(row: &pb::NewsletterMessage) -> &pb::InboundMessage {
    row.message.as_ref().expect("every row carries its message")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn core_reads_the_captured_page() {
    let mock = MockWaServer::start().await.expect("start mock");
    let prefix = common::test_prefix("stress_newsletter_history", "core_reads_the_captured_page");
    let logged = common::logged_in_client(&mock, &prefix).await;
    let client = logged.client.clone();
    let rows = core_reads(&client, &mock, captured_rows()).await;

    let ids: Vec<u64> = rows.iter().map(|r| r.server_id).collect();
    assert_eq!(
        ids,
        vec![773, 777, 780, 781, 782, 783, 784],
        "the row without server_id is skipped"
    );
    let types: Vec<&str> = rows.iter().map(|r| r.r#type.as_str()).collect();
    assert_eq!(
        types,
        vec!["media", "poll", "text", "text", "text", "hologram", "text"]
    );

    // The point of #26: a row lands in the shape the event bus delivers.
    let first = inbound(&rows[0]);
    assert_eq!(first.text, "bom dia");
    assert_eq!(first.chat, CHANNEL);
    assert_eq!(first.timestamp, 1_790_001_172_000, "seconds become ms");
    assert_eq!(first.raw_message, text_payload("bom dia"));
    let key = first.key.as_ref().expect("the key is always built");
    assert_eq!(
        (key.remote_jid.as_str(), key.id.as_str()),
        (CHANNEL, "3AEC773")
    );
    assert!(!key.from_me);
    // The row names no sender, so nothing is invented beyond `from_me`.
    assert!(first.sender.is_empty() && first.push_name.is_empty());
    assert!(inbound(&rows[6]).key.as_ref().expect("key").from_me);

    // The server-side tallies, the reason this RPC exists.
    assert_eq!(rows[0].forwards_count, 8329);
    let reactions: Vec<(&str, u64)> = rows[0]
        .reactions
        .iter()
        .map(|r| (r.code.as_str(), r.count))
        .collect();
    assert_eq!(
        reactions,
        vec![("\u{1F621}", 437)],
        "a code-less count drops"
    );
    assert_eq!(rows[1].poll_type, "creation");
    assert_eq!(rows[1].votes.len(), 1);
    assert_eq!(
        (rows[1].votes[0].option_hash.clone(), rows[1].votes[0].count),
        (option_hash(), 25328)
    );

    // #44: the revoked rows say so; an undecodable body is only empty.
    assert_eq!((rows[2].edit.as_str(), rows[3].edit.as_str()), ("8", "8"));
    assert_eq!(rows[4].edit, "");
    for revoked_or_broken in &rows[2..=4] {
        assert!(inbound(revoked_or_broken).raw_message.is_empty());
    }

    // #51: the edit times, by value, in ms; an unedited row carries neither.
    assert_eq!(rows[6].edit, "3");
    assert_eq!(
        (rows[6].original_timestamp, rows[6].last_edit_timestamp),
        (1_790_001_172_000, 1_790_004_321_987)
    );
    assert_eq!(
        (rows[0].original_timestamp, rows[0].last_edit_timestamp),
        (0, 0)
    );
    logged.cleanup().await;
}

// #1548 → #1558: an absent `type` relays as absence, not as the `text` the
// library's typed field defaults to.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn core_relays_an_absent_type_as_absent() {
    let mock = MockWaServer::start().await.expect("start mock");
    let prefix = common::test_prefix(
        "stress_newsletter_history",
        "core_relays_an_absent_type_as_absent",
    );
    let logged = common::logged_in_client(&mock, &prefix).await;
    let client = logged.client.clone();
    let rows = vec![
        row("790", None)
            .children([plaintext(text_payload("sem tipo"))])
            .build(),
    ];
    let out = core_reads(&client, &mock, rows).await;
    assert_eq!(out[0].r#type, "");
    logged.cleanup().await;
}

// #1548 → #1558: every `<meta polltype>` token relays, whether or not the
// library has a stage for it and whatever the row type. The edge decides.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn core_relays_every_polltype_token() {
    let mock = MockWaServer::start().await.expect("start mock");
    let prefix = common::test_prefix(
        "stress_newsletter_history",
        "core_relays_every_polltype_token",
    );
    let logged = common::logged_in_client(&mock, &prefix).await;
    let client = logged.client.clone();
    let rows = vec![
        row("791", Some("poll"))
            .children([meta_polltype("future_stage"), plaintext(text_payload("?"))])
            .build(),
        row("792", Some("text"))
            .children([meta_polltype("creation"), plaintext(text_payload("?"))])
            .build(),
    ];
    let out = core_reads(&client, &mock, rows).await;
    assert_eq!(out[0].poll_type, "future_stage");
    assert_eq!(out[1].poll_type, "creation");
    logged.cleanup().await;
}

// #43: "the server sent no count" is not "nobody picked this", and bytes that
// are not 32 long name no option.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn core_skips_a_vote_without_a_count_or_a_32_byte_hash() {
    let mock = MockWaServer::start().await.expect("start mock");
    let prefix = common::test_prefix(
        "stress_newsletter_history",
        "core_skips_a_vote_without_a_count_or_a_32_byte_hash",
    );
    let logged = common::logged_in_client(&mock, &prefix).await;
    let client = logged.client.clone();
    let rows = vec![
        row("793", Some("poll"))
            .children([
                meta_polltype("creation"),
                NodeBuilder::new("votes")
                    .children([
                        vote(Some("5"), vec![7; 31]),
                        vote(None, option_hash()),
                        vote(Some("9"), option_hash()),
                    ])
                    .build(),
                plaintext(text_payload("?")),
            ])
            .build(),
    ];
    let out = core_reads(&client, &mock, rows).await;
    let votes: Vec<(usize, u64)> = out[0]
        .votes
        .iter()
        .map(|v| (v.option_hash.len(), v.count))
        .collect();
    assert_eq!(votes, vec![(32, 9)], "only the well-formed vote crosses");
    logged.cleanup().await;
}

// #56: accepted change. The old path read an answer without `<messages>` as an
// empty page; WA Web's parser requires the node and throws, and so does the
// library. It is a failed read, not a channel with no history.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn core_fails_an_answer_without_messages() {
    let mock = MockWaServer::start().await.expect("start mock");
    let prefix = common::test_prefix(
        "stress_newsletter_history",
        "core_fails_an_answer_without_messages",
    );
    let logged = common::logged_in_client(&mock, &prefix).await;
    let client = logged.client.clone();
    mock.answer_newsletter_iq_with(NodeBuilder::new("unexpected").build());

    let err = newsletters::get_messages(&client, &history_request(20))
        .await
        .expect_err("an answer without <messages> is not an empty page");
    assert_eq!(tonic::Status::from(err).code(), tonic::Code::Unavailable);
    logged.cleanup().await;
}

// A page with `<messages>` and no rows is what an empty channel looks like.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn core_reads_an_empty_page_as_no_rows() {
    let mock = MockWaServer::start().await.expect("start mock");
    let prefix = common::test_prefix(
        "stress_newsletter_history",
        "core_reads_an_empty_page_as_no_rows",
    );
    let logged = common::logged_in_client(&mock, &prefix).await;
    let client = logged.client.clone();
    let out = core_reads(&client, &mock, Vec::new()).await;
    assert!(out.is_empty());
    logged.cleanup().await;
}
