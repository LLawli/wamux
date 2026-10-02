//! SIGTERM must end the daemon (#35). Measured on the production relay: with a
//! `SubscribeEvents` stream open, the daemon logged "received SIGTERM" and never
//! returned from `serve`, until systemd SIGKILLed it 90 s later and left the
//! socket file behind. tonic waits, with no deadline, for every connection to
//! close, and an event subscription is a stream that never ends on its own.
//!
//! Drives the real Unix-socket path on SQLite, so it needs no container.

use std::sync::Arc;
use std::time::Duration;

use tokio_stream::StreamExt;

use wamux::config::Config;
use wamux::proto::v1 as pb;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::server::Drained;
use wamux::state::{AccountRegistry, RegistryTuning};
use wamux::transport::shutdown::Shutdown;
use wamux::{server, transport};

#[allow(dead_code)]
mod common;

fn subscribe_all() -> pb::SubscribeRequest {
    pb::SubscribeRequest {
        selector: Some(pb::subscribe_request::Selector::AllAccounts(pb::Empty {})),
        replay_from_ring: 0,
    }
}

/// Grace far above the 5 s assertion: passing on the backstop would hide a
/// regression in the clean path, so the test demands `Drained::Clean`.
const GRACE: Duration = Duration::from_secs(30);

#[tokio::test]
async fn sigterm_ends_the_server_while_a_subscription_is_open() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("wamux.sock");
    let socket_str = socket.to_str().unwrap().to_string();
    let (engine, _db) = common::sqlite_engine().await;
    let registry = Arc::new(AccountRegistry::new(engine, RegistryTuning::with_ring(8)));
    // One account before subscribing, so the stream carries both kinds of
    // forwarder: the per-account one and the follower of new accounts.
    let prefix = common::test_prefix("graceful_shutdown", "sigterm_ends_the_server");
    common::sweep_orphans(registry.storage(), &prefix).await;
    let probe = registry
        .create_account(Some(&format!("{prefix}probe")))
        .await
        .unwrap();
    let config = Config {
        socket_path: socket_str.clone(),
        enable_reflection: false,
        ..Config::default()
    };

    let incoming = transport::uds_listener::bind(&socket_str, 0o660, None).expect("bind");
    let shutdown = Shutdown::new();
    let router = server::build_router(registry.clone(), &config, shutdown.clone());
    let server = tokio::spawn(server::serve_until_shutdown(
        router,
        incoming,
        shutdown.clone(),
        GRACE,
    ));

    // An all-accounts subscription: the stream the local mirror and the edge
    // hold open for the daemon's whole life.
    let mut client = EventServiceClient::new(common::uds_channel(&socket).await);
    let mut events = client
        .subscribe_events(subscribe_all())
        .await
        .expect("subscribe")
        .into_inner();

    shutdown.trigger();

    let finished = tokio::time::timeout(Duration::from_secs(5), server).await;
    let drained = finished
        .expect("the server was still serving 5 s after shutdown: an open subscription pins it")
        .expect("server task panicked")
        .expect("serve failed");
    assert_eq!(
        drained,
        Drained::Clean,
        "the subscription must end on its own, not be cut off by the grace"
    );

    // And the subscriber saw a clean end of stream, not a torn connection, so
    // an edge can tell "the daemon is restarting" from "the network broke".
    let end = tokio::time::timeout(Duration::from_secs(1), events.next()).await;
    assert!(
        matches!(end, Ok(None)),
        "expected a clean end of stream, got {end:?}"
    );

    registry
        .delete(&probe)
        .await
        .expect("delete the probe account");
}
