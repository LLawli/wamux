//! Issue #38 (item 3): why `domain/newsletters.rs` issues the channel MEX
//! queries itself instead of calling `client.newsletter().list_subscribed()` /
//! `.get_metadata()`. Both reach the server fine since upstream #1378; what the
//! library does with the ANSWER is the problem, and these tests pin it.
//!
//! The mock stands in for the server: it answers the MEX IQ with the JSON a
//! test chooses, and a real, unmodified `whatsapp-rust` Client parses it, the
//! same path production takes. The node is the answer captured from the live
//! server on the production accounts, with only the enum fields swapped.
//!
//! Two kinds of test live here:
//! - `core_*`: what this relay promises (server tokens relayed verbatim,
//!   a missing channel is NotFound, one bad node does not sink the list);
//! - `library_*`: a canary on upstream oxidezap/whatsapp-rust#1546. When it
//!   fails, the library stopped folding the state: re-read the note at the top
//!   of `domain/newsletters.rs` and decide whether the hand-rolled path can go.
//!
//! Run with: `cargo test --features stress --test stress_newsletter_parse`.
//! Requires the docker Postgres (DATABASE_URL).
#![cfg(feature = "stress")]

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use wamux::domain::newsletters;
use wamux::state::AccountRegistry;
use wamux::stress::MockWaServer;
use whatsapp_rust::features::NewsletterState;
use whatsapp_rust::{Client, Jid};

#[allow(dead_code)]
mod common;

const CHANNEL: &str = "120363144038483540@newsletter";

/// The live-captured channel node (see the fixture in `domain/newsletters.rs`),
/// with the three enum fields in the server's own spelling.
fn channel_node(state: &str, verification: &str, role: &str) -> Value {
    json!({
        "id": CHANNEL,
        "state": { "type": state },
        "thread_metadata": {
            "creation_time": "1688746895",
            "name": { "text": "WhatsApp" },
            "subscribers_count": "4242",
            "verification": verification
        },
        "viewer_metadata": { "role": role }
    })
}

/// Answer one `get_metadata` query the way the server does.
fn answer_one_channel(mock: &MockWaServer, node: Value) {
    mock.answer_mex_with(&json!({ "data": { "xwa2_newsletter": node } }).to_string());
}

/// A client logged in against the mock. Not `wait_for_connected`: its Ready
/// stage waits on post-login syncs the mock does not serve, and logged in is
/// all a MEX query needs. The registry is returned so it outlives the test.
async fn logged_in_client(mock: &MockWaServer) -> (Arc<AccountRegistry>, Arc<Client>) {
    let (registry, handle) = common::registered_account(mock.ws_url(), "stress-nl").await;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn core_relays_the_servers_tokens_verbatim() {
    let mock = MockWaServer::start().await.expect("start mock");
    let (_registry, client) = logged_in_client(&mock).await;

    let cases = [
        ("ACTIVE", "VERIFIED", "SUBSCRIBER"),
        ("SUSPENDED", "VERIFIED", "SUBSCRIBER"),
        ("GEOSUSPENDED", "UNVERIFIED", "ADMIN"),
        // Values no enum anywhere models: relayed, not folded.
        ("ARCHIVED", "PENDING_REVIEW", "MODERATOR"),
    ];
    for (state, verification, role) in cases {
        answer_one_channel(&mock, channel_node(state, verification, role));
        let out = newsletters::get_metadata(&client, CHANNEL)
            .await
            .expect("core get_metadata");
        assert_eq!(out.state, state.to_lowercase(), "state {state}");
        assert_eq!(out.verification, verification.to_lowercase());
        assert_eq!(out.role, role.to_lowercase());
        assert_eq!(out.name, "WhatsApp");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn core_answers_not_found_for_a_missing_channel() {
    let mock = MockWaServer::start().await.expect("start mock");
    let (_registry, client) = logged_in_client(&mock).await;

    answer_one_channel(&mock, Value::Null);
    let err = newsletters::get_metadata(&client, CHANNEL)
        .await
        .expect_err("a null answer is a missing channel");
    // Through the library this is an InvalidRequest, which `client_err` maps
    // to Unavailable: "the core is down" for a channel that does not exist.
    assert_eq!(tonic::Status::from(err).code(), tonic::Code::NotFound);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn core_keeps_the_list_when_one_node_is_broken() {
    let mock = MockWaServer::start().await.expect("start mock");
    let (_registry, client) = logged_in_client(&mock).await;

    let mut broken = channel_node("ACTIVE", "VERIFIED", "SUBSCRIBER");
    broken.as_object_mut().expect("node object").remove("id");
    let list = json!([channel_node("ACTIVE", "VERIFIED", "SUBSCRIBER"), broken]);
    mock.answer_mex_with(&json!({ "data": { "xwa2_newsletter_subscribed": list } }).to_string());

    let out = newsletters::list_subscribed(&client)
        .await
        .expect("core list_subscribed");
    let jids: Vec<&str> = out.newsletters.iter().map(|n| n.jid.as_str()).collect();
    assert_eq!(jids, vec![CHANNEL, ""], "the valid channel survives");

    // The library fails the whole call on the one node without an id.
    assert!(client.newsletter().list_subscribed().await.is_err());
}

/// Canary on upstream #1546: the library matches the state in lowercase while
/// the server sends uppercase, so a suspended channel reads as Active. The
/// lowercase control proves the arm exists and only the case misses it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn library_still_reads_every_uppercase_state_as_active() {
    let mock = MockWaServer::start().await.expect("start mock");
    let (_registry, client) = logged_in_client(&mock).await;
    let jid: Jid = CHANNEL.parse().expect("channel jid");

    for (sent, expected) in [
        ("SUSPENDED", NewsletterState::Active),
        ("GEOSUSPENDED", NewsletterState::Active),
        ("suspended", NewsletterState::Suspended),
    ] {
        answer_one_channel(&mock, channel_node(sent, "VERIFIED", "SUBSCRIBER"));
        let meta = client
            .newsletter()
            .get_metadata(&jid)
            .await
            .expect("library get_metadata");
        assert_eq!(
            meta.state, expected,
            "state {sent}: if this changed, upstream #1546 moved; see the note \
             at the top of domain/newsletters.rs"
        );
    }
}
