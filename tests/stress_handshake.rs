//! M1 of the stress harness: prove a real `whatsapp-rust` Client completes the
//! Noise XX handshake against our mock WhatsApp server over a `ws://` loopback.
//! Run with: `cargo test --features stress --test stress_handshake`.
//! Requires the docker Postgres (DATABASE_URL).
#![cfg(feature = "stress")]

use std::sync::Arc;
use std::time::Duration;

use wacore::store::Device;
use wamux::proto::v1::event_envelope::Event as WireEvent;
use wamux::state::{AccountRegistry, RegistryTuning};
use wamux::stress::MockWaServer;
use wamux::stress::mock_wa_server::{PUSHED_RECEIPT_FROM, PUSHED_RECEIPT_ID};

// Only a subset of the shared helpers is used per test binary.
#[allow(dead_code)]
mod common;

/// Honor `RUST_LOG` if set (so a diagnostic run can crank up whatsapp-rust /
/// keepalive logging), else a quiet default. Idempotent across tests.
fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn,wamux=info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

/// Build a registry pointed at the mock and create an account whose device is
/// already *registered* (pn set + persisted), so `connect` makes the client
/// send a LOGIN payload and treat the mock's `<success>` as auth success.
/// Returns the registry and the (not-yet-connected) account handle.
async fn registered_account(
    mock: &MockWaServer,
    tag_prefix: &str,
) -> (Arc<AccountRegistry>, Arc<wamux::state::AccountHandle>) {
    let engine = common::pg_engine(4).await;

    let tuning = RegistryTuning {
        ws_url_override: Some(mock.ws_url()),
        ..RegistryTuning::default()
    };
    let registry = Arc::new(AccountRegistry::new(engine.clone(), tuning));

    let tag = uuid::Uuid::new_v4();
    let handle = registry
        .create_account(Some(&format!("{tag_prefix}-{tag}")))
        .await
        .expect("create account");

    let mut device = Device::new();
    device.pn = Some(
        "5511999999999@s.whatsapp.net"
            .parse()
            .expect("parse pn jid"),
    );
    device.push_name = "Stress".to_string();
    registry
        .storage()
        .device_backend(handle.device_id)
        .save(&device)
        .await
        .expect("save registered device");

    (registry, handle)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_client_completes_handshake_against_mock() {
    init_tracing();

    let mock = MockWaServer::start().await.expect("start mock");
    let ws_url = mock.ws_url();

    let engine = common::pg_engine(4).await;

    let tuning = RegistryTuning {
        ws_url_override: Some(ws_url),
        ..RegistryTuning::default()
    };
    let registry = Arc::new(AccountRegistry::new(engine, tuning));

    let tag = uuid::Uuid::new_v4();
    let handle = registry
        .create_account(Some(&format!("stress-m1-{tag}")))
        .await
        .expect("create account");

    // Drive the bot; the Noise handshake runs in its run loop against the mock.
    registry
        .connect(&handle, None, true)
        .await
        .expect("connect");

    // Wait for the server to complete one handshake (decrypted ClientFinish).
    let mut completed = false;
    for _ in 0..100 {
        if mock.handshakes_completed() >= 1 {
            completed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    registry.disconnect(&handle).await;
    let _ = registry.delete(&handle).await;

    assert!(
        completed,
        "mock server should complete the XX handshake with a real client"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registered_client_logs_in_and_talks_over_transport() {
    init_tracing();

    let mock = MockWaServer::start().await.expect("start mock");
    let (registry, handle) = registered_account(&mock, "stress-m2").await;

    registry
        .connect(&handle, None, true)
        .await
        .expect("connect");

    // The client should accept <success>, log in, and send post-login IQs over
    // the encrypted transport — which the server decrypts AND parses (both
    // directions work). We wait on `parsed_nodes`, not just `post_login_frames`:
    // a decrypted-but-unparsed frame (the B1 flag-byte bug) would still bump the
    // frame count, so asserting on parsed nodes is what catches that regression.
    let mut parsed = false;
    for _ in 0..100 {
        if mock.parsed_nodes() >= 1 {
            parsed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    registry.disconnect(&handle).await;
    let _ = registry.delete(&handle).await;

    assert!(
        mock.post_login_frames() >= 1,
        "logged-in client should send decryptable post-login frames"
    );
    assert!(
        parsed,
        "the server must actually parse a post-login node, not just decrypt bytes"
    );
}

/// M2b: a server-pushed `<receipt>` must surface as a `ReceiptEvent` on the
/// account's broadcast — i.e. a pushed stanza flows through the real client's
/// node pipeline into wamux's event bridge, unchanged.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pushed_receipt_surfaces_as_event() {
    init_tracing();

    let mock = MockWaServer::start().await.expect("start mock");
    let (registry, handle) = registered_account(&mock, "stress-m2b-rcpt").await;

    // Subscribe BEFORE connecting: the broadcast has no replay, so a late
    // subscriber would miss the receipt the mock pushes right after login.
    let mut events = handle.subscribe();

    registry
        .connect(&handle, None, true)
        .await
        .expect("connect");

    // Wait for the Receipt envelope (ignore connection/other events in between).
    let mut got = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(1), events.recv()).await {
            Ok(Ok(env)) => {
                if let Some(WireEvent::Receipt(r)) = env.event {
                    got = Some(r);
                    break;
                }
            }
            Ok(Err(_)) => break, // channel closed
            Err(_) => continue,  // 1 s tick, keep waiting until the deadline
        }
    }

    registry.disconnect(&handle).await;
    let _ = registry.delete(&handle).await;

    let receipt = got.expect("pushed <receipt> should surface as a ReceiptEvent");
    assert_eq!(receipt.chat, PUSHED_RECEIPT_FROM, "receipt chat (from)");
    assert!(
        receipt.message_ids.iter().any(|id| id == PUSHED_RECEIPT_ID),
        "receipt should carry the pushed message id, got {:?}",
        receipt.message_ids
    );
}

/// M2b (longevity, `#[ignore]`): hold one connection idle long enough for the
/// client's 15-30 s keepalive loop to fire, prove the mock answers the ping, and
/// confirm the connection never reconnected (one handshake, still Connected).
/// Slow (~35 s) so it's opt-in: `cargo test --features stress -- --ignored`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "slow: waits out the 15-30 s keepalive interval"]
async fn connection_survives_keepalive_window() {
    init_tracing();

    let mock = MockWaServer::start().await.expect("start mock");
    let (registry, handle) = registered_account(&mock, "stress-m2b-keepalive").await;

    registry
        .connect(&handle, None, true)
        .await
        .expect("connect");

    // KEEP_ALIVE_INTERVAL_MAX is 30 s; give a margin for the ping to land.
    let mut pinged = false;
    for _ in 0..70 {
        if mock.keepalive_pings() >= 1 {
            pinged = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // The mock answers every <iq>, so the keepalive succeeds and the dead-socket
    // watchdog never trips: exactly one handshake, no silent reconnect.
    let handshakes = mock.handshakes_completed();

    registry.disconnect(&handle).await;
    let _ = registry.delete(&handle).await;

    assert!(
        pinged,
        "client should send a keepalive ping within the window"
    );
    assert_eq!(
        handshakes, 1,
        "connection must be sustained without reconnecting (one handshake only)"
    );
}

/// M3 (scale, `#[ignore]`): provision N registered accounts and connect them all
/// to the mock at once, proving the harness holds N live transport connections —
/// every real client completes the XX handshake, logs in, and stays supervised
/// (the M2b unpack fix is what lets all N survive their keepalive). Default
/// N=199; override with `STRESS_ACCOUNTS`. Opt-in (slow, many sockets):
/// `cargo test --features stress -- --ignored connect_many`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "scale/stress: provisions and connects ~199 real clients"]
async fn connect_many_clients_against_mock() {
    init_tracing();

    let n: usize = std::env::var("STRESS_ACCOUNTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(199);

    let mock = MockWaServer::start().await.expect("start mock");

    let engine = common::pg_engine(16).await;

    // Bounded graceful stop keeps the N-account teardown from dragging.
    let tuning = RegistryTuning {
        ws_url_override: Some(mock.ws_url()),
        graceful_stop_timeout: Duration::from_millis(500),
        ..RegistryTuning::default()
    };
    let registry = Arc::new(AccountRegistry::new(engine.clone(), tuning));

    // Provision N registered devices (pn set + persisted) so each connects via a
    // LOGIN payload and treats the mock's `<success>` as auth success.
    let tag = uuid::Uuid::new_v4();
    let mut handles = Vec::with_capacity(n);
    for i in 0..n {
        let handle = registry
            .create_account(Some(&format!("stress-m3-{tag}-{i}")))
            .await
            .expect("create account");
        let mut device = Device::new();
        device.pn = Some(
            format!("5511{:09}@s.whatsapp.net", 100_000_000 + i)
                .parse()
                .expect("parse pn jid"),
        );
        device.push_name = "Stress".to_string();
        registry
            .storage()
            .device_backend(handle.device_id)
            .save(&device)
            .await
            .expect("save registered device");
        handles.push(handle);
    }

    // Connect them all. `connect` returns once the run loop is spawned, so the N
    // XX handshakes then proceed concurrently in the background.
    for handle in &handles {
        registry.connect(handle, None, true).await.expect("connect");
    }

    // Wait until every client has completed its handshake against the mock.
    let mut completed = 0usize;
    for _ in 0..600 {
        completed = mock.handshakes_completed();
        if completed >= n {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert!(
        completed >= n,
        "all {n} clients should complete the handshake, got {completed}"
    );
    assert_eq!(
        registry.connected_count(),
        n,
        "all {n} accounts should be held connected after handshake"
    );
    assert!(
        mock.post_login_frames() >= n,
        "every client should send at least one decryptable post-login frame"
    );

    // Hold briefly and re-check: no client terminally exited — the harness
    // sustains N live connections concurrently (the M3 deliverable).
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        registry.connected_count(),
        n,
        "connections must stay live (no terminal exits under load)"
    );

    for handle in &handles {
        let _ = registry.delete(handle).await;
    }
}
