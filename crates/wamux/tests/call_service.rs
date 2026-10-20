//! CallService through the socket (#200): `RejectCall` over a real Unix socket
//! and tonic client, with the account logged in against `MockWaServer`. The
//! stanza shape is whatsapp-rust 6f07e3a's `build_reject`. Honors
//! `WAMUX_TEST_ENGINE`; `scripts/ci.sh` runs it on both engines.
#![cfg(feature = "stress")]

use tonic::Code;
use tonic::transport::Channel;
use wacore_binary::Node;
use wamux::proto::v1 as pb;
use wamux::proto::v1::call_service_client::CallServiceClient;
use wamux::stress::MockWaServer;

// Only a subset of the shared helpers is used per test binary.
#[allow(dead_code)]
mod common;

use common::mock_wire::{account_ref, attr};

const CALL_ID: &str = "9F4C2B7E1A3D5C8E0B6A4F2D1C3E5A7B";
/// The `<call>` wrapper's sender: one device of the caller.
const PEER: &str = "169815004184633:12@lid";
/// The action's `call-creator`: differs from `PEER` under companion signaling.
const CREATOR: &str = "5511999000111@s.whatsapp.net";

struct Fixture {
    mock: MockWaServer,
    logged: common::LoggedIn,
    calls: CallServiceClient<Channel>,
    account: pb::AccountRef,
}

async fn fixture(test: &str) -> Fixture {
    let mock = MockWaServer::start().await.expect("start mock");
    let prefix = common::test_prefix("call_service", test);
    let logged = common::logged_in_client(&mock, &prefix).await;
    let channel = common::serve_registry(logged.registry.clone()).await;
    let account = account_ref(&logged.handle.uuid.to_string());
    Fixture {
        mock,
        logged,
        calls: CallServiceClient::new(channel),
        account,
    }
}

fn jid(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

fn reject(
    account: &pb::AccountRef,
    call_id: &str,
    peer: Option<pb::Jid>,
    creator: Option<pb::Jid>,
) -> pb::RejectCallRequest {
    pb::RejectCallRequest {
        account: Some(account.clone()),
        call_id: call_id.to_string(),
        peer,
        call_creator: creator,
    }
}

/// The `<call>` stanzas the client sent so far.
fn sent_calls(mock: &MockWaServer) -> Vec<Node> {
    mock.client_stanzas("call")
}

/// The first `<call>` the client sent, waiting for it to reach the mock.
async fn first_call(mock: &MockWaServer) -> Node {
    common::poll_until(
        "the mock to receive a <call>",
        std::time::Duration::from_secs(5),
        || async { sent_calls(mock).into_iter().next() },
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reject_call_sends_the_reject_stanza_with_the_three_identifiers() {
    let mut f = fixture("reject_call_sends_the_reject_stanza_with_the_three_identifiers").await;
    let answer: pb::Empty = f
        .calls
        .reject_call(reject(&f.account, CALL_ID, jid(PEER), jid(CREATOR)))
        .await
        .expect("reject")
        .into_inner();
    assert_eq!(answer, pb::Empty {});
    let call = first_call(&f.mock).await;
    assert_eq!(
        attr(&call, "to").as_deref(),
        Some(PEER),
        "the reject goes back to the peer"
    );
    assert!(
        attr(&call, "id").is_some(),
        "the wrapper carries an id: {call:?}"
    );
    let children = call.children().expect("<call> carries an action");
    assert_eq!(children.len(), 1, "one action: {call:?}");
    let action = &children[0];
    assert_eq!(action.tag.as_ref(), "reject");
    assert_eq!(attr(action, "call-id").as_deref(), Some(CALL_ID));
    assert_eq!(attr(action, "call-creator").as_deref(), Some(CREATOR));
    assert_eq!(attr(action, "count").as_deref(), Some("0"));
    f.logged.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reject_call_refuses_an_empty_call_id_before_the_wire() {
    let mut f = fixture("reject_call_refuses_an_empty_call_id_before_the_wire").await;
    let status = f
        .calls
        .reject_call(reject(&f.account, "", jid(PEER), jid(CREATOR)))
        .await
        .expect_err("an empty call_id");
    assert_eq!(status.code(), Code::InvalidArgument, "{status:?}");
    assert_eq!(status.message(), "missing call_id");
    assert!(sent_calls(&f.mock).is_empty(), "no <call> stanza");
    f.logged.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reject_call_refuses_a_missing_or_malformed_peer_or_creator() {
    let mut f = fixture("reject_call_refuses_a_missing_or_malformed_peer_or_creator").await;
    let cases = [
        ("peer unset", None, jid(CREATOR), "missing jid"),
        (
            "peer malformed",
            jid("not a jid"),
            jid(CREATOR),
            "invalid jid 'not a jid'",
        ),
        ("creator unset", jid(PEER), None, "missing jid"),
        (
            "creator malformed",
            jid(PEER),
            jid("not a jid"),
            "invalid jid 'not a jid'",
        ),
    ];
    for (case, peer, creator, message) in cases {
        let status = f
            .calls
            .reject_call(reject(&f.account, CALL_ID, peer, creator))
            .await
            .expect_err(case);
        assert_eq!(status.code(), Code::InvalidArgument, "{case}: {status:?}");
        assert!(
            status.message().starts_with(message),
            "{case}: {}",
            status.message()
        );
    }
    assert!(sent_calls(&f.mock).is_empty(), "no <call> stanza");
    f.logged.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reject_call_answers_not_found_for_an_unknown_account() {
    let mut f = fixture("reject_call_answers_not_found_for_an_unknown_account").await;
    let unknown = account_ref("00000000-0000-4000-8000-000000000200");
    let status = f
        .calls
        .reject_call(reject(&unknown, CALL_ID, jid(PEER), jid(CREATOR)))
        .await
        .expect_err("an unknown account");
    assert_eq!(status.code(), Code::NotFound, "{status:?}");
    assert!(sent_calls(&f.mock).is_empty(), "no <call> stanza");
    f.logged.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reject_call_answers_failed_precondition_when_not_connected() {
    let mut f = fixture("reject_call_answers_failed_precondition_when_not_connected").await;
    let prefix = common::test_prefix("call_service", "reject_call_not_connected_idle");
    common::sweep_orphans(f.logged.registry.storage(), &prefix).await;
    let idle = f
        .logged
        .registry
        .create_account(Some(&format!("{prefix}idle")))
        .await
        .expect("idle account");
    let idle_ref = account_ref(&idle.uuid.to_string());
    let status = f
        .calls
        .reject_call(reject(&idle_ref, CALL_ID, jid(PEER), jid(CREATOR)))
        .await
        .expect_err("an account with no live client");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    f.logged
        .registry
        .delete(&idle)
        .await
        .expect("delete the idle account");
    f.logged.cleanup().await;
}
