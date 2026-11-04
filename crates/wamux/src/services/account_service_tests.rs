//! #247: what ends a pairing forwarder, each cause on its own. Over the socket
//! these hide behind each other: a parked forwarder holds no connection once
//! its client left, and nothing but a channel notices it is still there.

use std::time::Duration;

use tokio::sync::{broadcast, mpsc};
use tonic::Status;

use super::forward_pairing_updates;
use crate::proto::v1 as pb;
use crate::proto::v1::pairing_update::Event as Pairing;
use crate::transport::shutdown::Shutdown;

const SOON: Duration = Duration::from_secs(2);
/// Long enough for the spawned forwarder to reach its blocked `send`.
const SETTLE: Duration = Duration::from_millis(100);

fn envelope(event: Option<pb::event_envelope::Event>) -> pb::EventEnvelope {
    pb::EventEnvelope {
        account_uuid: String::new(),
        monotonic_seq: 0,
        ts_unix_ms: 0,
        event,
    }
}

fn update(event: Pairing) -> pb::PairingUpdate {
    pb::PairingUpdate { event: Some(event) }
}

fn pairing(event: Pairing) -> pb::EventEnvelope {
    envelope(Some(pb::event_envelope::Event::Pairing(update(event))))
}

fn qr() -> Pairing {
    Pairing::QrCode("qr".to_string())
}

/// `events_tx` outlives the forwarder and nothing is published, so the only
/// thing that can end it is the closed channel.
#[tokio::test]
async fn a_client_that_hung_up_ends_the_forwarder_with_no_event() {
    let (events_tx, events) = broadcast::channel(8);
    let (tx, rx) = mpsc::channel::<Result<pb::PairingUpdate, Status>>(1);
    drop(rx);

    let ended = tokio::time::timeout(SOON, forward_pairing_updates(events, tx, Shutdown::new()));

    ended
        .await
        .expect("the forwarder stayed parked after its client hung up");
    drop(events_tx);
}

/// A client that stopped reading is still connected, so `tx.closed()` stays
/// quiet: only a select around the `send` as well as the `recv` gets out.
#[tokio::test]
async fn shutdown_cuts_a_send_blocked_on_a_client_that_stopped_reading() {
    let (events_tx, events) = broadcast::channel(8);
    let (tx, _client) = mpsc::channel(1);
    tx.send(Ok(update(qr()))).await.expect("fill the channel");
    events_tx.send(pairing(qr())).expect("publish");
    let shutdown = Shutdown::new();
    let mut forwarder = tokio::spawn(forward_pairing_updates(events, tx, shutdown.clone()));

    let blocked = tokio::time::timeout(SETTLE, &mut forwarder).await;
    assert!(blocked.is_err(), "the forwarder must be stuck on the send");
    shutdown.trigger();

    tokio::time::timeout(SOON, forwarder)
        .await
        .expect("shutdown must cut a blocked send")
        .expect("forwarder panicked");
}

/// Updates cross in order, anything that is not a pairing event is skipped, and
/// the terminal update is the last thing the client sees.
#[tokio::test]
async fn updates_reach_the_client_until_the_terminal_one() {
    let (events_tx, events) = broadcast::channel(8);
    let (tx, mut client) = mpsc::channel(8);
    let terminal = Pairing::Error(pb::PairingError {
        message: "refused".to_string(),
    });
    events_tx.send(envelope(None)).expect("publish");
    events_tx.send(pairing(qr())).expect("publish");
    events_tx.send(pairing(terminal.clone())).expect("publish");
    events_tx.send(pairing(qr())).expect("publish");

    let ended = forward_pairing_updates(events, tx, Shutdown::new());
    tokio::time::timeout(SOON, ended)
        .await
        .expect("the terminal update must end the forwarder");

    let next = client.recv().await.expect("the QR").expect("not a Status");
    assert_eq!(next, update(qr()));
    let last = client
        .recv()
        .await
        .expect("the error")
        .expect("not a Status");
    assert_eq!(last, update(terminal));
    assert!(client.recv().await.is_none(), "nothing after the terminal");
}
