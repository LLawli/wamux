//! A subscriber that joins with a replay in the middle of a burst gets every
//! envelope of the burst, once and in `monotonic_seq` order (#213).
//!
//! The replay dedupe skips every live envelope at or below the highest seq it
//! replayed. When the stream could reach the ring out of order, an envelope
//! stamped before that highest seq but pushed after the snapshot was skipped
//! live and never replayed: lost by the core, not by the subscriber. The
//! burst goes through the account's own publisher, the path the library's
//! events take. Drives `EventSvc` directly, like `event_subscription.rs`.
//!
//! Runs against whichever engine WAMUX_TEST_ENGINE names (default Postgres,
//! which needs DATABASE_URL and the docker container).

use std::sync::Arc;
use std::time::Duration;

use tokio_stream::StreamExt;
use tonic::Request;
use wacore::types::events::{Event, Receipt};
use wacore::types::message::MessageSource;
use wacore::types::presence::ReceiptType;

use wamux::proto::v1 as pb;
use wamux::proto::v1::event_service_server::EventService;
use wamux::services::event_service::EventSvc;
use wamux::state::{AccountHandle, AccountRegistry, RegistryTuning};

// Only a subset of the shared helpers is used per test binary.
#[allow(dead_code)]
mod common;

/// Big enough that one task per event inverted in every round measured; the
/// ring and the broadcast both hold it whole, so no overflow or lag can stand
/// in for a lost envelope.
const BURST: usize = 2000;
/// Each round joins at a different point of a fresh burst.
const ROUNDS: usize = 10;

fn receipt(id: usize) -> Arc<Event> {
    let source = MessageSource {
        chat: "5511999990000@s.whatsapp.net".parse().unwrap(),
        sender: "5511999990000@s.whatsapp.net".parse().unwrap(),
        is_from_me: false,
        ..Default::default()
    };
    Arc::new(Event::Receipt(
        Receipt::builder()
            .source(source)
            .message_ids(vec![format!("ID{id}").into()])
            .timestamp(wacore::time::from_secs(1_717_932_111).unwrap())
            .r#type(ReceiptType::Delivered)
            .offline(false)
            .build(),
    ))
}

fn replaying(handle: &AccountHandle) -> pb::SubscribeRequest {
    pb::SubscribeRequest {
        selector: Some(pb::subscribe_request::Selector::Account(pb::AccountRef {
            r#ref: Some(pb::account_ref::Ref::Uuid(handle.uuid.to_string())),
        })),
        replay_from_ring: (BURST * 2) as u32,
    }
}

/// Publish a burst on `handle`, subscribe with a replay once `join_at` of it
/// is enqueued, and return the seqs that subscriber received.
async fn seqs_seen_joining_at(
    svc: &EventSvc,
    handle: Arc<AccountHandle>,
    join_at: usize,
) -> Vec<i64> {
    let (joined_tx, joined_rx) = tokio::sync::oneshot::channel();
    let publisher = handle.publisher.clone();
    let burst = tokio::spawn(async move {
        for id in 0..join_at {
            publisher.publish_lib(receipt(id));
        }
        let _ = joined_tx.send(());
        for id in join_at..BURST {
            publisher.publish_lib(receipt(id));
            if id % 64 == 0 {
                tokio::task::yield_now().await;
            }
        }
    });
    joined_rx.await.expect("burst started");
    let mut stream = svc
        .subscribe_events(Request::new(replaying(&handle)))
        .await
        .expect("subscribe with replay")
        .into_inner();
    burst.await.expect("burst task");

    let mut seqs = Vec::with_capacity(BURST);
    while seqs.len() < BURST {
        match tokio::time::timeout(Duration::from_secs(5), stream.next()).await {
            Ok(Some(Ok(envelope))) => seqs.push(envelope.monotonic_seq),
            // A short stream is the failure this test is about: report what
            // arrived instead of panicking on the timeout.
            _ => break,
        }
    }
    seqs
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replay_during_a_burst_loses_no_event() {
    let engine = common::test_engine().await;
    let tuning = RegistryTuning {
        ring_capacity: BURST * 2,
        broadcast_capacity: BURST * 2,
        ..RegistryTuning::default()
    };
    let registry = Arc::new(AccountRegistry::new(engine, tuning));
    let prefix = common::test_prefix("event_order", "replay_during_a_burst");
    let _ = common::sweep_orphans(registry.storage(), &prefix).await;
    let svc = EventSvc::new(registry.clone());

    let expected: Vec<i64> = (0..BURST as i64).collect();
    for round in 0..ROUNDS {
        let tag = uuid::Uuid::new_v4();
        let handle = registry
            .create_account(Some(&format!("{prefix}{tag}")))
            .await
            .expect("create account");
        let join_at = BURST / 4 + round * (BURST / (2 * ROUNDS));
        let seqs = seqs_seen_joining_at(&svc, handle.clone(), join_at).await;
        registry.delete(&handle).await.expect("delete account");
        let missing: Vec<i64> = expected
            .iter()
            .copied()
            .filter(|seq| !seqs.contains(seq))
            .collect();
        assert!(
            missing.is_empty(),
            "round {round}, joined at {join_at}: {} envelope(s) never arrived, \
             first {:?}",
            missing.len(),
            &missing[..missing.len().min(10)]
        );
        assert_eq!(seqs, expected, "round {round}: in seq order, each once");
    }
    let _ = common::sweep_orphans(registry.storage(), &prefix).await;
}
