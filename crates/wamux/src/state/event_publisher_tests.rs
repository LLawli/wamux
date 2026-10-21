//! The publisher is the one writer of an account's stream (#213): whatever
//! order events reach it in, subscribers and the replay ring see the
//! sequence in that same order, with no inversion and no gap.
//!
//! The bursts are sized so that the delivery this replaces (one task per
//! event, racing between stamping the seq and broadcasting it) inverts with
//! near certainty: measured at about 4.6% of envelopes in a 5000-event burst
//! on a 4-worker runtime.

use std::sync::Arc;
use std::sync::atomic::AtomicI64;

use tokio::sync::{broadcast, watch};
use wacore::types::events::{Event, Receipt};
use wacore::types::message::MessageSource;
use wacore::types::presence::ReceiptType;

use super::EventPublisher;
use crate::proto::v1::event_envelope::Event as WireEvent;
use crate::proto::v1::{ConnectionState, EventEnvelope, RawEvent};
use crate::state::event_bridge::EventCtx;
use crate::state::event_ring::EventRing;

const BURST: usize = 5000;

struct Bench {
    publisher: EventPublisher,
    ring: Arc<EventRing>,
    rx: broadcast::Receiver<EventEnvelope>,
}

/// A publisher whose broadcast and ring both hold a whole burst, so neither a
/// lagged receiver nor a ring overflow can stand in for a lost event.
fn bench(capacity: usize) -> Bench {
    let (events_tx, rx) = broadcast::channel(capacity);
    let (state_tx, _) = watch::channel(ConnectionState::Disconnected);
    let ring = Arc::new(EventRing::new(capacity));
    let publisher = EventPublisher::spawn(EventCtx {
        uuid: "account".to_string(),
        events_tx,
        ring: ring.clone(),
        state_tx: Arc::new(state_tx),
        seq: Arc::new(AtomicI64::new(0)),
        replay_max_event_bytes: 0,
    });
    Bench {
        publisher,
        ring,
        rx,
    }
}

/// A library event that carries `id` through the mapping (as its one message
/// id), so a test can tell which publish an envelope came from.
fn receipt(id: &str) -> Arc<Event> {
    let source = MessageSource {
        chat: "5511999990000@s.whatsapp.net".parse().unwrap(),
        sender: "5511999990000@s.whatsapp.net".parse().unwrap(),
        is_from_me: false,
        ..Default::default()
    };
    Arc::new(Event::Receipt(
        Receipt::builder()
            .source(source)
            .message_ids(vec![id.into()])
            .timestamp(wacore::time::from_secs(1_717_932_111).unwrap())
            .r#type(ReceiptType::Delivered)
            .offline(false)
            .build(),
    ))
}

/// A relay-made event, as the send echo is, carrying `kind` as its label.
fn wire(kind: &str) -> WireEvent {
    WireEvent::Raw(RawEvent {
        kind: kind.to_string(),
        payload: Vec::new(),
        note: String::new(),
    })
}

/// The label a test put on an envelope: the receipt's id or the raw kind.
fn label(envelope: &EventEnvelope) -> String {
    match &envelope.event {
        Some(WireEvent::Receipt(receipt)) => receipt.message_ids.join(","),
        Some(WireEvent::Raw(raw)) => raw.kind.clone(),
        other => panic!("unexpected event in a test burst: {other:?}"),
    }
}

async fn receive(rx: &mut broadcast::Receiver<EventEnvelope>, count: usize) -> Vec<EventEnvelope> {
    let mut received = Vec::with_capacity(count);
    for _ in 0..count {
        let next = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await;
        received.push(
            next.expect("an envelope in time")
                .expect("no lag, no close"),
        );
    }
    received
}

fn seqs(envelopes: &[EventEnvelope]) -> Vec<i64> {
    envelopes.iter().map(|e| e.monotonic_seq).collect()
}

fn zero_to(count: usize) -> Vec<i64> {
    (0..count as i64).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn burst_of_library_events_arrives_in_seq_order() {
    let mut bench = bench(BURST * 2);
    for i in 0..BURST {
        bench.publisher.publish_lib(receipt(&format!("ID{i}")));
    }
    let received = receive(&mut bench.rx, BURST).await;
    assert_eq!(
        seqs(&received),
        zero_to(BURST),
        "every seq from 0, once, in increasing order"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn seq_follows_library_dispatch_order() {
    let mut bench = bench(BURST * 2);
    for i in 0..BURST {
        bench.publisher.publish_lib(receipt(&format!("ID{i}")));
    }
    let received = receive(&mut bench.rx, BURST).await;
    for envelope in &received {
        assert_eq!(
            label(envelope),
            format!("ID{}", envelope.monotonic_seq),
            "the n-th event the library dispatched is stamped seq n"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ring_keeps_seq_order_after_a_burst() {
    let mut bench = bench(BURST * 2);
    for i in 0..BURST {
        bench.publisher.publish_lib(receipt(&format!("ID{i}")));
    }
    // The last broadcast follows the last ring push, so the ring is complete.
    receive(&mut bench.rx, BURST).await;
    let ring = bench.ring.snapshot(BURST * 2).await;
    assert_eq!(
        seqs(&ring),
        zero_to(BURST),
        "the replay ring holds the burst in seq order"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wire_event_is_in_the_ring_when_publish_returns() {
    let bench = bench(BURST * 2);
    for i in 0..BURST {
        bench.publisher.publish_lib(receipt(&format!("ID{i}")));
    }
    bench.publisher.publish_wire(wire("echo")).await;
    // No waiting on the broadcast: the return of publish_wire is the promise.
    let ring = bench.ring.snapshot(BURST * 2).await;
    assert_eq!(
        ring.last().map(label).as_deref(),
        Some("echo"),
        "the echo is in the ring, after everything enqueued before it"
    );
    assert_eq!(seqs(&ring), zero_to(BURST + 1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wire_and_library_events_share_one_sequence() {
    const ECHOES: usize = 500;
    let mut bench = bench(BURST * 2);
    let echoes: Vec<_> = (0..ECHOES)
        .map(|i| {
            let publisher = bench.publisher.clone();
            tokio::spawn(async move { publisher.publish_wire(wire(&format!("echo{i}"))).await })
        })
        .collect();
    for i in 0..BURST {
        bench.publisher.publish_lib(receipt(&format!("ID{i}")));
    }
    for echo in echoes {
        echo.await.expect("echo task");
    }
    let received = receive(&mut bench.rx, BURST + ECHOES).await;
    assert_eq!(
        seqs(&received),
        zero_to(BURST + ECHOES),
        "one sequence across both paths, delivered in order"
    );
    let library: Vec<String> = received
        .iter()
        .map(label)
        .filter(|l| l.starts_with("ID"))
        .collect();
    let expected: Vec<String> = (0..BURST).map(|i| format!("ID{i}")).collect();
    assert_eq!(
        library, expected,
        "library events keep their dispatch order around the echoes"
    );
    let mut echo_labels: Vec<String> = received
        .iter()
        .map(label)
        .filter(|l| l.starts_with("echo"))
        .collect();
    echo_labels.sort();
    let mut expected_echoes: Vec<String> = (0..ECHOES).map(|i| format!("echo{i}")).collect();
    expected_echoes.sort();
    assert_eq!(echo_labels, expected_echoes, "every echo, exactly once");
}
