//! Sprint 5 regression: the all-accounts subscription is DYNAMIC — accounts
//! created AFTER subscribing deliver into an already-open stream (the gap
//! recorded in docs/STATUS.md §4), and the snapshot path (accounts existing
//! before the subscribe) keeps delivering. Drives `EventSvc` directly (no
//! socket), mirroring tests/load_multi_account.rs.
//!
//! Requires the docker Postgres (DATABASE_URL).

use std::sync::Arc;
use std::time::Duration;

use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Status};

use wamux::proto::v1 as pb;
use wamux::proto::v1::event_service_server::EventService;
use wamux::services::event_service::EventSvc;
use wamux::state::{AccountRegistry, RegistryTuning};

// Only a subset of the shared helpers is used per test binary.
#[allow(dead_code)]
mod common;

fn subscribe_all() -> pb::SubscribeRequest {
    pb::SubscribeRequest {
        selector: Some(pb::subscribe_request::Selector::AllAccounts(pb::Empty {})),
        replay_from_ring: 0,
    }
}

/// Wait (bounded) until the account's broadcast has a live receiver: forwarder
/// attachment is async, and a `broadcast::send` with zero receivers is lost,
/// so pushing before attachment would race. Polling the receiver count is
/// deterministic where a fixed sleep would be flaky.
async fn await_forwarder_attached(
    events_tx: &tokio::sync::broadcast::Sender<pb::EventEnvelope>,
    which: &str,
) {
    for _ in 0..200 {
        if events_tx.receiver_count() > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no forwarder attached to the {which} account within 2s");
}

async fn expect_event_from(
    stream: &mut ReceiverStream<Result<pb::EventEnvelope, Status>>,
    account_uuid: &str,
    seq: i64,
) {
    match tokio::time::timeout(Duration::from_secs(5), stream.next()).await {
        Ok(Some(Ok(envelope))) => {
            assert_eq!(envelope.account_uuid, account_uuid);
            assert_eq!(envelope.monotonic_seq, seq);
        }
        other => panic!("expected event from account {account_uuid}: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_accounts_subscription_includes_later_created_accounts() {
    let engine = common::pg_engine(4).await;
    let registry = Arc::new(AccountRegistry::new(engine, RegistryTuning::with_ring(8)));
    let swept = common::sweep_orphans(registry.storage(), "evsub-").await;
    if swept > 0 {
        eprintln!("(swept {swept} orphan evsub- account(s) from a prior run)");
    }

    // One account exists BEFORE the subscribe: covers the snapshot path.
    let tag = uuid::Uuid::new_v4();
    let before = registry
        .create_account(Some(&format!("evsub-{tag}-before")))
        .await
        .expect("create before-account");

    let svc = EventSvc::new(registry.clone());
    let mut stream = svc
        .subscribe_events(Request::new(subscribe_all()))
        .await
        .expect("subscribe all-accounts")
        .into_inner();

    // Snapshot path intact: the pre-existing account still delivers.
    await_forwarder_attached(&before.events_tx, "before").await;
    let before_uuid = before.uuid.to_string();
    let _ = before
        .events_tx
        .send(common::synthetic_envelope(&before_uuid, 1, 0));
    expect_event_from(&mut stream, &before_uuid, 1).await;

    // Dynamic path: an account created AFTER the subscribe delivers too —
    // before Sprint 5 it never got a forwarder (and with zero accounts at
    // subscribe time the stream would already be closed).
    let after = registry
        .create_account(Some(&format!("evsub-{tag}-after")))
        .await
        .expect("create after-account");
    await_forwarder_attached(&after.events_tx, "after").await;
    let after_uuid = after.uuid.to_string();
    let _ = after
        .events_tx
        .send(common::synthetic_envelope(&after_uuid, 2, 0));
    expect_event_from(&mut stream, &after_uuid, 2).await;

    // Tidy: proper deletes through the registry, then the sweep as a backstop
    // (an aborted run is caught by the next run's setup-sweep).
    registry.delete(&before).await.expect("delete before");
    registry.delete(&after).await.expect("delete after");
    let _ = common::sweep_orphans(registry.storage(), "evsub-").await;
}

// Regression (code-review 2026-06-11): an event dispatched between
// CreateAccount and the dynamic forwarder's subscribe used to vanish from
// replay_from_ring=0 all-accounts streams (broadcast to zero receivers). The
// dispatch path pushes to the ring BEFORE broadcasting and the
// created-follower replays a created account's FULL ring, so the event is
// recovered — and forward()'s seq filter keeps it exactly-once when the live
// copy also lands. Distinct prefix: the other test's setup-sweep runs in
// parallel and must not reap this test's rows.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn created_account_event_before_attach_is_recovered_from_ring() {
    let engine = common::pg_engine(4).await;
    let registry = Arc::new(AccountRegistry::new(engine, RegistryTuning::with_ring(8)));
    let _ = common::sweep_orphans(registry.storage(), "evsubring-").await;

    let svc = EventSvc::new(registry.clone());
    let mut stream = svc
        .subscribe_events(Request::new(subscribe_all()))
        .await
        .expect("subscribe all-accounts")
        .into_inner();

    let tag = uuid::Uuid::new_v4();
    let account = registry
        .create_account(Some(&format!("evsubring-{tag}")))
        .await
        .expect("create account");
    let uuid = account.uuid.to_string();
    // Mimic event_bridge::dispatch firing before the forwarder attached:
    // ring first, then a broadcast that may find zero receivers (= dropped).
    account
        .ring
        .push(common::synthetic_envelope(&uuid, 1, 0))
        .await;
    let _ = account
        .events_tx
        .send(common::synthetic_envelope(&uuid, 1, 0));

    expect_event_from(&mut stream, &uuid, 1).await;

    // Exactly-once: the replayed and live copies must not both surface.
    let extra = tokio::time::timeout(Duration::from_millis(300), stream.next()).await;
    assert!(extra.is_err(), "event delivered twice: {extra:?}");

    registry.delete(&account).await.expect("delete account");
    let _ = common::sweep_orphans(registry.storage(), "evsubring-").await;
}
