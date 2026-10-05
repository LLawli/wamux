//! Channel metadata through the library (issue #56): what `ListSubscribedNewsletters`
//! and `GetNewsletterMetadata` relay for the answers the server really sends.
//!
//! The mock stands in for the server: it answers the MEX IQ with the JSON a
//! test chooses, and a real, unmodified `whatsapp-rust` Client parses it, the
//! same path production takes. The channel node is the answer captured from
//! the live server on the production accounts, with only the enum fields
//! swapped; the missing-channel node was captured live through WA Web.
//!
//! - `core_*`: what this relay promises;
//! - `accepted_*`: where the library still answers less than the server sent,
//!   accepted on purpose and measured at zero rows in production (see the note
//!   at the top of `domain/newsletters.rs`). When one fails, upstream started
//!   keeping the distinction: relay it.
//!
//! Run with: `cargo test --features stress --test stress_newsletter_parse`.
//! Requires the docker Postgres (DATABASE_URL).
#![cfg(feature = "stress")]

use serde_json::{Value, json};
use wamux::domain::newsletters;
use wamux::stress::MockWaServer;
use wamux_types::Jid;

#[allow(dead_code)]
mod common;

const CHANNEL: &str = "120363144038483540@newsletter";

/// The channel as the domain takes it since #116: parsed at the boundary.
fn channel() -> Jid {
    Jid::parse(CHANNEL).expect("channel jid")
}

/// The live-captured channel node, with the three enum fields in the server's
/// own spelling.
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

/// What the server answered on 2026-09-28 for a well-formed newsletter JID
/// with no channel behind it (upstream #1560): not `null`.
fn non_existing_node() -> Value {
    json!({
        "id": null,
        "state": { "type": "NON_EXISTING" },
        "thread_metadata": null,
        "viewer_metadata": null
    })
}

/// Answer one `get_metadata` query the way the server does.
fn answer_one_channel(mock: &MockWaServer, node: Value) {
    mock.answer_mex_with(&json!({ "data": { "xwa2_newsletter": node } }).to_string());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn core_relays_the_servers_tokens_lowercased() {
    let mock = MockWaServer::start().await.expect("start mock");
    let prefix = common::test_prefix(
        "stress_newsletter_parse",
        "core_relays_the_servers_tokens_lowercased",
    );
    let logged = common::logged_in_client(&mock, &prefix).await;
    let client = logged.client.clone();

    let cases = [
        ("ACTIVE", "VERIFIED", "SUBSCRIBER"),
        ("SUSPENDED", "VERIFIED", "SUBSCRIBER"),
        ("GEOSUSPENDED", "UNVERIFIED", "ADMIN"),
        // A state and a verification no enum models: kept, not folded (#1557).
        ("ARCHIVED", "PENDING_REVIEW", "OWNER"),
    ];
    for (state, verification, role) in cases {
        answer_one_channel(&mock, channel_node(state, verification, role));
        let out = newsletters::get_metadata(&client, &channel())
            .await
            .expect("core get_metadata");
        assert_eq!(out.state, state.to_lowercase(), "state {state}");
        assert_eq!(out.verification, verification.to_lowercase());
        assert_eq!(out.role, role.to_lowercase());
        assert_eq!(out.jid, CHANNEL);
        assert_eq!(out.name, "WhatsApp");
    }
    logged.cleanup().await;
}

// #56: the test used to answer `null` here, which is not what the server sends,
// and so it never saw the core relay the live answer as an empty Newsletter.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn core_answers_not_found_for_a_missing_channel() {
    let mock = MockWaServer::start().await.expect("start mock");
    let prefix = common::test_prefix(
        "stress_newsletter_parse",
        "core_answers_not_found_for_a_missing_channel",
    );
    let logged = common::logged_in_client(&mock, &prefix).await;
    let client = logged.client.clone();

    for (label, node) in [("live", non_existing_node()), ("null", Value::Null)] {
        answer_one_channel(&mock, node);
        let err = newsletters::get_metadata(&client, &channel())
            .await
            .expect_err("a missing channel is not a Newsletter");
        assert_eq!(
            tonic::Status::from(err).code(),
            tonic::Code::NotFound,
            "{label}"
        );
    }
    logged.cleanup().await;
}

// #56: an entry that stands for no channel is dropped, as WA Web drops it; the
// valid ones survive. It used to relay as a Newsletter with `jid: ""`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn core_skips_a_list_entry_without_an_id() {
    let mock = MockWaServer::start().await.expect("start mock");
    let prefix = common::test_prefix(
        "stress_newsletter_parse",
        "core_skips_a_list_entry_without_an_id",
    );
    let logged = common::logged_in_client(&mock, &prefix).await;
    let client = logged.client.clone();

    let mut id_less = channel_node("ACTIVE", "VERIFIED", "SUBSCRIBER");
    id_less.as_object_mut().expect("node object").remove("id");
    let list = json!([
        channel_node("ACTIVE", "VERIFIED", "SUBSCRIBER"),
        id_less,
        non_existing_node(),
        Value::Null
    ]);
    mock.answer_mex_with(&json!({ "data": { "xwa2_newsletter_subscribed": list } }).to_string());

    let out = newsletters::list_subscribed(&client)
        .await
        .expect("core list_subscribed");
    let jids: Vec<&str> = out.newsletters.iter().map(|n| n.jid.as_str()).collect();
    assert_eq!(jids, vec![CHANNEL]);
    logged.cleanup().await;
}

// #56: measured in production on a channel the account does not follow. A MEX
// refusal is the server's answer, not the core being down.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn core_relays_a_mex_refusal_as_the_servers_code() {
    let mock = MockWaServer::start().await.expect("start mock");
    let prefix = common::test_prefix(
        "stress_newsletter_parse",
        "core_relays_a_mex_refusal_as_the_servers_code",
    );
    let logged = common::logged_in_client(&mock, &prefix).await;
    let client = logged.client.clone();

    mock.answer_mex_with(
        &json!({
            "data": null,
            "errors": [{
                "message": "Not Allowed",
                "extensions": { "error_code": 405, "is_summary": true, "severity": "CRITICAL" }
            }]
        })
        .to_string(),
    );
    let status = tonic::Status::from(
        newsletters::get_metadata(&client, &channel())
            .await
            .expect_err("a refused query is an error"),
    );
    assert_eq!(status.code(), tonic::Code::PermissionDenied);
    assert_eq!(status.metadata().get("wa-code").expect("wa-code"), "405");
    logged.cleanup().await;
}

// Accepted loss 1: a role the library has no variant for relays as absent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_an_unmodelled_role_relays_as_empty() {
    let mock = MockWaServer::start().await.expect("start mock");
    let prefix = common::test_prefix(
        "stress_newsletter_parse",
        "accepted_an_unmodelled_role_relays_as_empty",
    );
    let logged = common::logged_in_client(&mock, &prefix).await;
    let client = logged.client.clone();

    answer_one_channel(&mock, channel_node("ACTIVE", "VERIFIED", "MODERATOR"));
    let out = newsletters::get_metadata(&client, &channel())
        .await
        .expect("core get_metadata");
    assert_eq!(
        out.role, "",
        "upstream now keeps an unknown role: relay the token"
    );
    logged.cleanup().await;
}

// Accepted loss 2: an absent state or verification reads as the library's
// default instead of as absent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_absent_state_and_verification_read_as_defaults() {
    let mock = MockWaServer::start().await.expect("start mock");
    let prefix = common::test_prefix(
        "stress_newsletter_parse",
        "accepted_absent_state_and_verification_read_as_defaults",
    );
    let logged = common::logged_in_client(&mock, &prefix).await;
    let client = logged.client.clone();

    let mut node = channel_node("ACTIVE", "VERIFIED", "SUBSCRIBER");
    node.as_object_mut().expect("node object").remove("state");
    node["thread_metadata"]
        .as_object_mut()
        .expect("thread object")
        .remove("verification");
    answer_one_channel(&mock, node);
    let out = newsletters::get_metadata(&client, &channel())
        .await
        .expect("core get_metadata");
    assert_eq!(
        (out.state.as_str(), out.verification.as_str()),
        ("active", "unverified"),
        "upstream now keeps absence: relay it as \"\""
    );
    logged.cleanup().await;
}
