//! The replay ring's entry cap (#80), through the real ring path a test can
//! drive: the echo every send publishes. `replay_max_event_bytes` now defaults
//! to 256 KiB, so an event above it still reaches the live subscribers but a
//! subscriber that reconnects and replays the ring does not get it back.
//! The fixture's registry is built with `RegistryTuning::default()`, the same
//! default the daemon's config has.
//!
//! The cap is on the encoded envelope, and an echo carries its text twice: in
//! `text` and inside `raw_message`. A text of N bytes is an envelope of about
//! 2N, so 300 KB of text is well over 256 KiB and 100 KB is well under.

use std::time::Duration;

use tonic::Streaming;
use wamux::proto::v1 as pb;

use crate::harness::{Fixture, fixture, send_text};

/// A bound on waiting for one more event; the echoes arrive in milliseconds.
const PATIENCE: Duration = Duration::from_secs(10);
const MARKER: &str = "#80 marker";

async fn subscribe(f: &Fixture, replay_from_ring: u32) -> Streaming<pb::EventEnvelope> {
    let request = pb::SubscribeRequest {
        selector: Some(pb::subscribe_request::Selector::Account(f.account.clone())),
        replay_from_ring,
    };
    f.events
        .clone()
        .subscribe_events(request)
        .await
        .expect("subscribe")
        .into_inner()
}

/// The text of the next echo of a send by this account, skipping other events.
async fn next_echo_text(stream: &mut Streaming<pb::EventEnvelope>) -> String {
    loop {
        let envelope = tokio::time::timeout(PATIENCE, stream.message())
            .await
            .expect("an event in time")
            .expect("the stream is healthy")
            .expect("the stream is open");
        let Some(pb::event_envelope::Event::Message(message)) = envelope.event else {
            continue;
        };
        if message.key.as_ref().is_some_and(|key| key.from_me) {
            return message.text;
        }
    }
}

/// The lengths of the echoes a replay hands back, up to and including MARKER.
async fn replayed_until_marker(f: &Fixture) -> Vec<usize> {
    let mut replay = subscribe(f, 256).await;
    let mut lengths = Vec::new();
    loop {
        let text = next_echo_text(&mut replay).await;
        if text == MARKER {
            return lengths;
        }
        lengths.push(text.len());
    }
}

/// Send `body`, see its echo arrive live, then send MARKER.
async fn send_seen_live(f: &mut Fixture, body: &str) {
    let mut live = subscribe(f, 0).await;
    let to = f.peer.pn();
    send_text(f, to.clone(), body).await;
    assert_eq!(
        next_echo_text(&mut live).await.len(),
        body.len(),
        "the live subscriber gets the echo whatever its size"
    );
    send_text(f, to, MARKER).await;
    assert_eq!(next_echo_text(&mut live).await, MARKER);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_event_over_the_default_cap_is_live_but_not_replayed() {
    let mut f = fixture("an_event_over_the_default_cap_is_live_but_not_replayed").await;
    let big = "a".repeat(300_000);
    send_seen_live(&mut f, &big).await;
    let replayed = replayed_until_marker(&f).await;
    assert!(
        !replayed.contains(&big.len()),
        "an event of about 600 KB stayed in the ring: {replayed:?}"
    );
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_event_under_the_cap_is_replayed() {
    let mut f = fixture("an_event_under_the_cap_is_replayed").await;
    let large = "b".repeat(100_000);
    send_seen_live(&mut f, &large).await;
    let replayed = replayed_until_marker(&f).await;
    assert!(
        replayed.contains(&large.len()),
        "an event of about 200 KB left the ring: {replayed:?}"
    );
    f.cleanup().await;
}
