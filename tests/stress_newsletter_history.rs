//! Issue #40: can `GetNewsletterMessages` drop its hand-rolled IQ for
//! `client.newsletter().get_messages()`? Upstream #1523 and #1518 fixed the two
//! reasons it was hand-rolled (addressing, children dropped), so what is left
//! is the check #38 ran on the metadata queries: does the library's parsed
//! `NewsletterMessage` keep everything this RPC relays?
//!
//! The mock answers the `newsletter` IQ with a page a test builds, and both
//! paths read the same bytes through a real Client: the core's
//! `newsletters::get_messages` and the library's `get_messages`. The page is
//! the live-captured shape from #26 plus the edge cases the issue lists.
//!
//! - `both_*`: where the two agree, so a swap would change nothing;
//! - `library_*`: where the library answers differently. Each one is a reason
//!   the hand-rolled path stays; when one fails, upstream changed and the note
//!   at the top of `domain/newsletters.rs` is due for a re-read.
//!
//! Run with: `cargo test --features stress --test stress_newsletter_history`.
//! Requires the docker Postgres (DATABASE_URL).
#![cfg(feature = "stress")]

use std::sync::Arc;
use std::time::Duration;

use wacore_binary::Node;
use wacore_binary::builder::NodeBuilder;
use wamux::domain::newsletters;
use wamux::proto::v1 as pb;
use wamux::state::AccountRegistry;
use wamux::stress::MockWaServer;
use whatsapp_rust::buffa::Message as _;
use whatsapp_rust::features::NewsletterMessage;
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::{Client, Jid};

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

/// The rows both paths read the same way (#26's capture, plus revoked and
/// undecodable bodies).
fn agreeing_rows() -> Vec<Node> {
    let hash = wacore::poll::compute_option_hash(POLL_OPTION).to_vec();
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
                    .children([vote(Some("25328"), hash)])
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
        // No server_id: nothing can address it, both skip it.
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

/// A client logged in against the mock. Same recipe as
/// `stress_newsletter_parse`: logged in is all an IQ needs.
async fn logged_in_client(mock: &MockWaServer) -> (Arc<AccountRegistry>, Arc<Client>) {
    let (registry, handle) = common::registered_account(mock.ws_url(), "stress-nl-hist").await;
    registry
        .connect(&handle, None, true)
        .await
        .expect("connect");
    let client = handle.client().await.expect("client after connect");
    for _ in 0..100 {
        if client.is_logged_in() {
            return (registry, client);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("client never logged in against the mock");
}

/// Both paths over the same page.
async fn read_both(
    client: &Client,
    mock: &MockWaServer,
    rows: Vec<Node>,
) -> (Vec<pb::NewsletterMessage>, Vec<NewsletterMessage>) {
    mock.answer_newsletter_iq_with(page(rows));
    let core = newsletters::get_messages(client, &history_request(20))
        .await
        .expect("core get_messages")
        .messages;
    let jid: Jid = CHANNEL.parse().expect("channel jid");
    let library = client
        .newsletter()
        .get_messages(jid, 20, None)
        .await
        .expect("library get_messages");
    (core, library)
}

/// server_id, type, payload, forwards_count, poll_type, edit.
type WireRow = (u64, String, Vec<u8>, u64, String, String);

/// The library's row in the core's wire terms, for the fields both carry.
fn library_row_as_wire(row: &NewsletterMessage) -> WireRow {
    (
        row.server_id,
        row.message_type.as_str().to_string(),
        row.message
            .as_ref()
            .map(|message| message.encode_to_vec())
            .unwrap_or_default(),
        row.forwards_count.unwrap_or(0),
        row.poll_type
            .map(|kind| kind.as_str().to_string())
            .unwrap_or_default(),
        row.edit.as_str().to_string(),
    )
}

fn core_row_as_wire(row: &pb::NewsletterMessage) -> WireRow {
    let raw = row
        .message
        .as_ref()
        .map(|message| message.raw_message.clone())
        .unwrap_or_default();
    (
        row.server_id,
        row.r#type.clone(),
        raw,
        row.forwards_count,
        row.poll_type.clone(),
        row.edit.clone(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn both_read_the_captured_page_the_same() {
    let mock = MockWaServer::start().await.expect("start mock");
    let (_registry, client) = logged_in_client(&mock).await;
    let (core, library) = read_both(&client, &mock, agreeing_rows()).await;

    assert_eq!(core.len(), 6, "the row without server_id is skipped");
    assert_eq!(core.len(), library.len());
    for (ours, theirs) in core.iter().zip(&library) {
        assert_eq!(core_row_as_wire(ours), library_row_as_wire(theirs));
        let reactions: Vec<(String, u64)> = theirs
            .reactions
            .iter()
            .map(|r| (r.code.clone(), r.count))
            .collect();
        let ours_reactions: Vec<(String, u64)> = ours
            .reactions
            .iter()
            .map(|r| (r.code.clone(), r.count))
            .collect();
        assert_eq!(ours_reactions, reactions, "row {}", ours.server_id);
        let votes: Vec<(Vec<u8>, u64)> = theirs
            .votes
            .iter()
            .map(|v| (v.option_hash.to_vec(), v.count))
            .collect();
        let ours_votes: Vec<(Vec<u8>, u64)> = ours
            .votes
            .iter()
            .map(|v| (v.option_hash.clone(), v.count))
            .collect();
        assert_eq!(ours_votes, votes, "row {}", ours.server_id);
    }
    // The two tallies #26 is for, checked by value and not only by agreement.
    assert_eq!(core[0].forwards_count, 8329);
    assert_eq!(core[1].votes[0].count, 25328);
    assert_eq!(core[1].votes[0].option_hash.len(), 32);
    // #44: the revoked rows say so, instead of reading as an undecodable body.
    assert_eq!((core[2].edit.as_str(), core[3].edit.as_str()), ("8", "8"));
    assert_eq!(core[4].edit, "");
    // An undecodable body is an empty payload on BOTH sides, with nothing
    // saying so: a swap would not fix that, and would not make it worse.
    assert!(library[4].message.is_none());
    assert!(
        core[4]
            .message
            .as_ref()
            .expect("row")
            .raw_message
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn library_invents_a_type_the_server_did_not_send() {
    let mock = MockWaServer::start().await.expect("start mock");
    let (_registry, client) = logged_in_client(&mock).await;
    let rows = vec![
        row("790", None)
            .children([plaintext(text_payload("sem tipo"))])
            .build(),
    ];
    let (core, library) = read_both(&client, &mock, rows).await;
    assert_eq!(core[0].r#type, "", "the core relays the absence");
    assert_eq!(library[0].message_type.as_str(), "text");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn library_drops_a_polltype_it_has_no_variant_for() {
    let mock = MockWaServer::start().await.expect("start mock");
    let (_registry, client) = logged_in_client(&mock).await;
    let rows = vec![
        row("791", Some("poll"))
            .children([meta_polltype("future_stage"), plaintext(text_payload("?"))])
            .build(),
        // A polltype on a row that is not a poll: WA Web ignores it, and so
        // does the library. The core relays it; the edge decides.
        row("792", Some("text"))
            .children([meta_polltype("creation"), plaintext(text_payload("?"))])
            .build(),
    ];
    let (core, library) = read_both(&client, &mock, rows).await;
    assert_eq!(core[0].poll_type, "future_stage");
    assert_eq!(library[0].poll_type, None);
    assert_eq!(core[1].poll_type, "creation");
    assert_eq!(library[1].poll_type, None);
}

// Issue #43: this was `library_skips_a_vote_the_core_relays` until the core
// stopped relaying a 31-byte hash and an absent count as a tally.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn both_skip_a_vote_without_a_count_or_a_32_byte_hash() {
    let mock = MockWaServer::start().await.expect("start mock");
    let (_registry, client) = logged_in_client(&mock).await;
    let hash = wacore::poll::compute_option_hash(POLL_OPTION).to_vec();
    let rows = vec![
        row("793", Some("poll"))
            .children([
                meta_polltype("creation"),
                NodeBuilder::new("votes")
                    .children([
                        vote(Some("5"), vec![7; 31]),
                        vote(None, hash.clone()),
                        vote(Some("9"), hash),
                    ])
                    .build(),
                plaintext(text_payload("?")),
            ])
            .build(),
    ];
    let (core, library) = read_both(&client, &mock, rows).await;
    let ours: Vec<(usize, u64)> = core[0]
        .votes
        .iter()
        .map(|v| (v.option_hash.len(), v.count))
        .collect();
    let theirs: Vec<(usize, u64)> = library[0]
        .votes
        .iter()
        .map(|v| (v.option_hash.len(), v.count))
        .collect();
    assert_eq!(ours, vec![(32, 9)], "only the well-formed vote crosses");
    assert_eq!(ours, theirs);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn library_fails_an_answer_without_messages() {
    let mock = MockWaServer::start().await.expect("start mock");
    let (_registry, client) = logged_in_client(&mock).await;
    mock.answer_newsletter_iq_with(NodeBuilder::new("unexpected").build());

    let core = newsletters::get_messages(&client, &history_request(20))
        .await
        .expect("the core reads a page it cannot find as empty");
    assert!(core.messages.is_empty());
    let jid: Jid = CHANNEL.parse().expect("channel jid");
    assert!(
        client
            .newsletter()
            .get_messages(jid, 20, None)
            .await
            .is_err(),
        "the library fails the call"
    );
}
