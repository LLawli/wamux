//! #108: a pairing binary sends only after the post-pairing reconnect.
//!
//! The daemon reports CONNECTED from `PairSuccess` on, and the library sends no
//! disconnect for the 515 that follows, so these tests feed the event sequence
//! a real pairing produces (Paired, nothing, CONNECTED) and check the wait
//! keys on the CONNECTED after Paired, never on the status.

use std::sync::Arc;
use std::time::Duration;

use wamux::proto::v1 as pb;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;
use wamux::state::AccountHandle;
use wamux_tools::post_pairing::{PairedReconnect, send_self_after_reconnect, wait_reconnected};
use wamux_tools::report::{Outcome, Report};
use wamux_tools::socket_client::{ClientError, account_ref, connect_uds, subscribe_account};

mod support;
use support::TestDaemon;

const SELF_JID: &str = "5500000000108@s.whatsapp.net";
const SHORT_WAIT: Duration = Duration::from_millis(300);
const LONG_WAIT: Duration = Duration::from_secs(5);

fn envelope(event: pb::event_envelope::Event) -> pb::EventEnvelope {
    pb::EventEnvelope {
        event: Some(event),
        ..Default::default()
    }
}

fn paired() -> pb::EventEnvelope {
    envelope(pb::event_envelope::Event::Pairing(pb::PairingUpdate {
        event: Some(pb::pairing_update::Event::Paired(pb::PairedInfo {
            jid: Some(pb::Jid {
                value: SELF_JID.to_string(),
            }),
            ..Default::default()
        })),
    }))
}

fn connection(state: pb::ConnectionState) -> pb::EventEnvelope {
    envelope(pb::event_envelope::Event::Connection(
        pb::ConnectionStateChanged {
            state: state as i32,
            ..Default::default()
        },
    ))
}

fn raw() -> pb::EventEnvelope {
    envelope(pb::event_envelope::Event::Raw(pb::RawEvent {
        kind: "unrelated".to_string(),
        ..Default::default()
    }))
}

#[test]
fn paired_reconnect_needs_connected_after_paired() {
    let mut watch = PairedReconnect::default();
    assert!(
        !watch.observe(&connection(pb::ConnectionState::Connected)),
        "a CONNECTED before Paired is the old connection, not the reconnect"
    );
    assert!(
        !watch.observe(&paired()),
        "Paired is the moment the 515 is still to come"
    );
    assert!(!watch.observe(&connection(pb::ConnectionState::Connecting)));
    assert!(!watch.observe(&connection(pb::ConnectionState::Disconnected)));
    assert!(
        watch.observe(&connection(pb::ConnectionState::Connected)),
        "CONNECTED after Paired is the reconnect"
    );
}

#[test]
fn paired_reconnect_ignores_other_events() {
    let mut watch = PairedReconnect::default();
    assert!(!watch.observe(&raw()));
    assert!(!watch.observe(&paired()));
    assert!(!watch.observe(&raw()));
    assert!(!watch.observe(&envelope(pb::event_envelope::Event::Pairing(
        pb::PairingUpdate { event: None }
    ))));
}

#[tokio::test]
async fn wait_reconnected_reports_an_ended_stream() {
    let mut events = tokio_stream::iter(vec![Ok::<_, String>(paired())]);
    let err = wait_reconnected(&mut events, LONG_WAIT).await.unwrap_err();
    assert!(matches!(err, ClientError::EventsEnded), "{err:?}");
}

#[tokio::test]
async fn wait_reconnected_skips_stream_errors() {
    let mut events = tokio_stream::iter(vec![
        Err("lagged 3".to_string()),
        Ok(paired()),
        Err("lagged 1".to_string()),
        Ok(connection(pb::ConnectionState::Connected)),
    ]);
    wait_reconnected(&mut events, LONG_WAIT)
        .await
        .expect("an error between events is not the end of the stream");
}

/// A daemon with one account and a live subscription to it, plus the handle
/// the test publishes that account's events through.
struct Subscribed {
    daemon: TestDaemon,
    handle: Arc<AccountHandle>,
    events: tonic::Streaming<pb::EventEnvelope>,
}

async fn subscribed(name: &str) -> Subscribed {
    let daemon = TestDaemon::start().await;
    let handle = daemon.registry.create_account(Some(name)).await.unwrap();
    let before = handle.events_tx.receiver_count();
    let channel = connect_uds(&daemon.socket_str()).await.unwrap();
    let events = subscribe_account(channel, &account_ref(name))
        .await
        .unwrap();
    // The daemon's forwarder subscribes from a spawned task: publishing before
    // it has would drop the event, so wait until it holds a receiver.
    tokio::time::timeout(LONG_WAIT, async {
        while handle.events_tx.receiver_count() <= before {
            // not a sync point: the poll interval of a wait bounded by the condition
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the daemon never subscribed the forwarder");
    Subscribed {
        daemon,
        handle,
        events,
    }
}

fn publish(handle: &AccountHandle, events: &[pb::EventEnvelope]) {
    for event in events {
        handle.events_tx.send(event.clone()).unwrap();
    }
}

#[tokio::test]
async fn subscribe_account_to_an_unknown_account_is_not_found() {
    let daemon = TestDaemon::start().await;
    let channel = connect_uds(&daemon.socket_str()).await.unwrap();
    let err = subscribe_account(channel, &account_ref("tools-108-nobody"))
        .await
        .unwrap_err();
    match err {
        ClientError::Rpc { status, .. } => assert_eq!(status.code(), tonic::Code::NotFound),
        other => panic!("expected the daemon's NotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn wait_reconnected_returns_on_connected_after_paired() {
    let mut sub = subscribed("tools-108-reconnects").await;
    publish(
        &sub.handle,
        &[paired(), connection(pb::ConnectionState::Connected)],
    );
    wait_reconnected(&mut sub.events, LONG_WAIT)
        .await
        .expect("CONNECTED after Paired over the socket is the reconnect");
}

#[tokio::test]
async fn wait_reconnected_times_out_without_the_reconnect() {
    let mut sub = subscribed("tools-108-stays-down").await;
    publish(
        &sub.handle,
        &[paired(), connection(pb::ConnectionState::Connecting)],
    );
    let err = wait_reconnected(&mut sub.events, SHORT_WAIT)
        .await
        .unwrap_err();
    match err {
        ClientError::NotReconnected { waited } => assert_eq!(waited, SHORT_WAIT),
        other => panic!("expected NotReconnected, got {other:?}"),
    }
}

fn self_message_checks(report: &Report) -> Vec<Outcome> {
    report
        .checks()
        .iter()
        .filter(|c| c.name == "Messaging.SendText(self)")
        .map(|c| c.outcome)
        .collect()
}

#[tokio::test]
async fn self_message_is_sent_after_the_reconnect() {
    let mut sub = subscribed("tools-108-sends").await;
    let channel = connect_uds(&sub.daemon.socket_str()).await.unwrap();
    let mut messaging = MessagingServiceClient::new(channel);
    let mut report = Report::new();
    publish(
        &sub.handle,
        &[paired(), connection(pb::ConnectionState::Connected)],
    );
    send_self_after_reconnect(
        &mut messaging,
        &mut sub.events,
        &mut report,
        account_ref("tools-108-sends"),
        SELF_JID.to_string(),
        LONG_WAIT,
    )
    .await;
    // The test account runs no bot, so the daemon refuses the send: what this
    // proves is that the send was made, once, after the reconnect.
    assert_eq!(
        self_message_checks(&report).len(),
        1,
        "{:?}",
        report.checks()
    );
}

#[tokio::test]
async fn self_message_is_skipped_not_failed_when_the_reconnect_times_out() {
    let mut sub = subscribed("tools-108-skips").await;
    let channel = connect_uds(&sub.daemon.socket_str()).await.unwrap();
    let mut messaging = MessagingServiceClient::new(channel);
    let mut report = Report::new();
    publish(&sub.handle, &[paired()]);
    send_self_after_reconnect(
        &mut messaging,
        &mut sub.events,
        &mut report,
        account_ref("tools-108-skips"),
        SELF_JID.to_string(),
        SHORT_WAIT,
    )
    .await;
    assert!(
        self_message_checks(&report).is_empty(),
        "a skipped note records no check: {:?}",
        report.checks()
    );
    assert_eq!(report.counts().fail, 0, "{:?}", report.checks());
}
