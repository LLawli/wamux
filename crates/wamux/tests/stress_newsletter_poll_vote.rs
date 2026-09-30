//! Issue #26: the channel poll vote, this account's own add-ons and the
//! live-update subscription, end to end through a real Client against the
//! mock. The vote asserts the stanza the core's call puts on the wire against
//! the one WA Web sent live (2026-09-25, WhatsApp channel, poll 777); the
//! reads feed the library the answers captured that day.
//!
//! Run with: `cargo test --features stress --test stress_newsletter_poll_vote`.
//! Requires the docker Postgres (DATABASE_URL).
#![cfg(feature = "stress")]

use std::sync::Arc;
use std::time::Duration;

use wacore_binary::Node;
use wacore_binary::builder::NodeBuilder;
use wamux::domain::newsletters;
use wamux::error::WamuxError;
use wamux::proto::v1 as pb;
use wamux::state::AccountRegistry;
use wamux::stress::MockWaServer;
use whatsapp_rust::Client;

#[allow(dead_code)]
mod common;

const CHANNEL: &str = "120363144038483540@newsletter";
const POLL_SERVER_ID: u64 = 777;

fn hash(option: &str) -> Vec<u8> {
    wacore::poll::compute_option_hash(option).to_vec()
}

fn good_morning() -> Vec<u8> {
    hash("\u{2600}\u{FE0F} GOOD MORNING EVERYONE LET'S GO!!!")
}

fn mondays() -> Vec<u8> {
    hash("\u{1F636}\u{200D}\u{1F32B}\u{FE0F} Mondays should be illegal.")
}

/// A client logged in against the mock. Same recipe as
/// `stress_newsletter_history`: logged in is all these calls need.
async fn logged_in_client(mock: &MockWaServer) -> (Arc<AccountRegistry>, Arc<Client>) {
    let (registry, handle) = common::registered_account(mock.ws_url(), "stress-nl-vote").await;
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

fn vote_request(option_hashes: Vec<Vec<u8>>) -> pb::SendNewsletterPollVoteRequest {
    pb::SendNewsletterPollVoteRequest {
        account: None,
        jid: CHANNEL.to_string(),
        server_id: POLL_SERVER_ID,
        option_hashes,
    }
}

/// The `<message>` the mock received with this id, waiting for it to land.
async fn sent_message(mock: &MockWaServer, id: &str) -> Node {
    for _ in 0..50 {
        let found = mock
            .client_messages()
            .into_iter()
            .find(|m| attr(m, "id").as_deref() == Some(id));
        if let Some(message) = found {
            return message;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the mock never received a <message id={id}>");
}

fn attr(node: &Node, key: &str) -> Option<String> {
    node.as_node_ref()
        .get_attr(key)
        .map(|v| v.as_str().into_owned())
}

/// The `<vote>` payloads under `<votes>`, in order.
fn vote_payloads(message: &Node) -> Vec<Vec<u8>> {
    let votes = message
        .get_optional_child_by_tag(&["votes"])
        .expect("<votes>");
    votes
        .get_children_by_tag("vote")
        .map(|v| match &v.content {
            Some(wacore_binary::NodeContent::Bytes(bytes)) => bytes.to_vec(),
            other => panic!("<vote> carries bytes, got {other:?}"),
        })
        .collect()
}

// The shape measured live: `<message to type="poll" id server_id>` with
// `<meta polltype="vote"/>` then `<votes>`, one `<vote>` per option, in order.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_vote_puts_the_measured_stanza_on_the_wire() {
    let mock = MockWaServer::start().await.expect("start mock");
    let (_registry, client) = logged_in_client(&mock).await;
    let answer =
        newsletters::send_poll_vote(&client, &vote_request(vec![good_morning(), mondays()]))
            .await
            .expect("vote");
    assert!(!answer.stanza_id.is_empty());

    let message = sent_message(&mock, &answer.stanza_id).await;
    assert_eq!(attr(&message, "to").as_deref(), Some(CHANNEL));
    assert_eq!(attr(&message, "type").as_deref(), Some("poll"));
    assert_eq!(attr(&message, "server_id").as_deref(), Some("777"));
    let children: Vec<&str> = message
        .children()
        .expect("children")
        .iter()
        .map(|c| c.tag.as_ref())
        .collect();
    assert_eq!(children, vec!["meta", "votes"]);
    let meta = message
        .get_optional_child_by_tag(&["meta"])
        .expect("<meta>");
    assert_eq!(attr(meta, "polltype").as_deref(), Some("vote"));
    assert_eq!(attr(meta, "contenttype"), None);
    assert_eq!(vote_payloads(&message), vec![good_morning(), mondays()]);
}

// Removing a vote is the same stanza with an empty `<votes/>`, as measured.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_selection_sends_an_empty_votes() {
    let mock = MockWaServer::start().await.expect("start mock");
    let (_registry, client) = logged_in_client(&mock).await;
    let answer = newsletters::send_poll_vote(&client, &vote_request(Vec::new()))
        .await
        .expect("remove the vote");
    let message = sent_message(&mock, &answer.stanza_id).await;
    assert!(vote_payloads(&message).is_empty());
}

// A malformed request is the caller's mistake: InvalidArgument, and nothing
// reaches the wire.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_malformed_vote_is_refused_before_anything_is_sent() {
    let mock = MockWaServer::start().await.expect("start mock");
    let (_registry, client) = logged_in_client(&mock).await;
    let short = vote_request(vec![vec![0u8; 31]]);
    let mut not_a_channel = vote_request(vec![good_morning()]);
    not_a_channel.jid = "120363041234567890@g.us".to_string();
    let mut no_poll = vote_request(vec![good_morning()]);
    no_poll.server_id = 0;
    for request in [short, not_a_channel, no_poll] {
        let err = newsletters::send_poll_vote(&client, &request)
            .await
            .expect_err("malformed");
        assert!(matches!(err, WamuxError::InvalidArgument(_)), "{err}");
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(mock.client_messages().is_empty(), "nothing may be sent");
}

/// `<my_addons>` as captured, plus a removed vote and a reaction.
fn my_addons_answer() -> Node {
    let vote = |bytes: Vec<u8>| NodeBuilder::new("vote").bytes(bytes).build();
    let message = |server_id: &str, child: Node| {
        NodeBuilder::new("message")
            .attr("server_id", server_id)
            .children([child])
            .build()
    };
    NodeBuilder::new("my_addons")
        .children([NodeBuilder::new("messages")
            .attr("jid", CHANNEL)
            .children([
                message(
                    "777",
                    NodeBuilder::new("votes")
                        .attr("t", "1790340039")
                        .children([vote(good_morning()), vote(mondays())])
                        .build(),
                ),
                message(
                    "769",
                    NodeBuilder::new("votes").attr("t", "1790340162").build(),
                ),
                message(
                    "778",
                    NodeBuilder::new("reaction")
                        .attr("code", "\u{1F44D}")
                        .attr("t", "1790340100")
                        .build(),
                ),
            ])
            .build()])
        .build()
}

// The server's record of this account's own vote, in ms. A removed vote is a
// present `poll_vote` with no hashes; a message with only a reaction has none.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn my_addons_relay_the_servers_record() {
    let mock = MockWaServer::start().await.expect("start mock");
    let (_registry, client) = logged_in_client(&mock).await;
    mock.answer_newsletter_iq_with(my_addons_answer());
    let request = pb::GetMyNewsletterAddOnsRequest {
        account: None,
        jid: CHANNEL.to_string(),
        limit: 20,
    };
    let list = newsletters::get_my_addons(&client, &request)
        .await
        .expect("my addons")
        .messages;
    assert_eq!(list.len(), 3);

    let voted = list[0].poll_vote.as_ref().expect("777 voted");
    assert_eq!(list[0].server_id, 777);
    assert_eq!(voted.timestamp, 1_790_340_039_000);
    assert_eq!(voted.option_hashes, vec![good_morning(), mondays()]);

    let removed = list[1].poll_vote.as_ref().expect("769 kept its removal");
    assert!(removed.option_hashes.is_empty());
    assert_eq!(removed.timestamp, 1_790_340_162_000);

    assert!(list[2].poll_vote.is_none());
    let reaction = list[2].reaction.as_ref().expect("778 reacted");
    assert_eq!(reaction.code, "\u{1F44D}");
    assert_eq!(reaction.timestamp, 1_790_340_100_000);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn my_addons_refuse_a_zero_limit() {
    let mock = MockWaServer::start().await.expect("start mock");
    let (_registry, client) = logged_in_client(&mock).await;
    let request = pb::GetMyNewsletterAddOnsRequest {
        account: None,
        jid: CHANNEL.to_string(),
        limit: 0,
    };
    let err = newsletters::get_my_addons(&client, &request)
        .await
        .expect_err("limit 0");
    assert!(matches!(err, WamuxError::InvalidArgument(_)), "{err}");
}

// The subscription answers the server's duration (90 s, measured live).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_updates_answer_the_servers_duration() {
    let mock = MockWaServer::start().await.expect("start mock");
    let (_registry, client) = logged_in_client(&mock).await;
    mock.answer_newsletter_iq_with(
        NodeBuilder::new("live_updates")
            .attr("duration", "90")
            .build(),
    );
    let answer = newsletters::subscribe_live_updates(&client, CHANNEL)
        .await
        .expect("subscribe");
    assert_eq!(answer.duration_seconds, 90);
}
