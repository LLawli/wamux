//! #252: `stop` ends a bot stuck in its connect handshake.
//!
//! The library awaits its transport and its Noise handshake (20 s each) without
//! racing them against shutdown, so a bot whose upstream accepts and never
//! answers cannot leave its run loop when `stop` asks. Detached past
//! `graceful_stop_timeout`, it held a budget slot for the rest of the attempt,
//! and a `connect` inside that window started a second bot that the first one's
//! supervisor later unhooked from the account: no `disconnect` could stop it.
//!
//! The upstream here is a listener the test owns, so it sees each bot's socket
//! open and close: a closed socket is the proof the bot is gone, not a flag the
//! account reports about itself. Honors `WAMUX_TEST_ENGINE`.

use crate::common;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use wamux::state::{AccountHandle, AccountRegistry, RegistryTuning};

/// Short, so the stop times out long before the library's own 20 s connect
/// timeout would end the attempt.
const GRACE: Duration = Duration::from_millis(300);
/// Far under the 20 s the stuck attempt lasts, far over an aborted bot's close
/// (0.82 s measured with a 300 ms grace).
const CLOSE_WAIT: Duration = Duration::from_secs(5);
/// How long a connect may take to dial the loopback listener.
const DIAL_WAIT: Duration = Duration::from_secs(10);

/// A listener that accepts and never answers. Each accepted socket reports
/// its open and, once the peer closes it, its close.
struct SilentUpstream {
    port: u16,
    opened: mpsc::UnboundedReceiver<usize>,
    closed: mpsc::UnboundedReceiver<usize>,
}

impl SilentUpstream {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind upstream");
        let port = listener.local_addr().expect("upstream address").port();
        let (opened_tx, opened) = mpsc::unbounded_channel();
        let (closed_tx, closed) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut index = 0;
            while let Ok((socket, _)) = listener.accept().await {
                let _ = opened_tx.send(index);
                tokio::spawn(report_close(socket, index, closed_tx.clone()));
                index += 1;
            }
        });
        Self {
            port,
            opened,
            closed,
        }
    }

    async fn wait_opened(&mut self, expected: usize) {
        let opened = tokio::time::timeout(DIAL_WAIT, self.opened.recv()).await;
        assert_eq!(
            opened.expect("the bot never dialed the upstream"),
            Some(expected),
            "unexpected socket order"
        );
    }

    async fn assert_closed(&mut self, expected: usize) {
        let closed = tokio::time::timeout(CLOSE_WAIT, self.closed.recv()).await;
        let closed = closed.unwrap_or_else(|_| {
            panic!("socket #{expected} still open {CLOSE_WAIT:?} after stop: the bot outlived it")
        });
        assert_eq!(closed, Some(expected), "a different socket closed first");
    }
}

/// Reads until the peer closes: nothing is ever written, so EOF (or a reset)
/// is the bot dropping its transport.
async fn report_close(
    mut socket: tokio::net::TcpStream,
    index: usize,
    closed: mpsc::UnboundedSender<usize>,
) {
    let mut buf = [0u8; 4096];
    while matches!(socket.read(&mut buf).await, Ok(read) if read > 0) {}
    let _ = closed.send(index);
}

async fn fixture(
    test: &str,
    upstream: &SilentUpstream,
) -> (Arc<AccountRegistry>, Arc<AccountHandle>) {
    let engine = common::test_engine().await;
    let prefix = common::test_prefix("stop_stuck_handshake", test);
    common::sweep_orphans(&engine, &prefix).await;
    let tuning = RegistryTuning {
        ws_url_override: Some(format!("ws://127.0.0.1:{}", upstream.port)),
        graceful_stop_timeout: GRACE,
        ..RegistryTuning::default()
    };
    let registry = Arc::new(AccountRegistry::new(engine, tuning));
    let handle = registry
        .create_account(Some(&format!("{prefix}{}", uuid::Uuid::new_v4())))
        .await
        .expect("create account");
    (registry, handle)
}

#[tokio::test]
async fn stop_ends_a_bot_stuck_in_its_handshake() {
    let mut upstream = SilentUpstream::start().await;
    let (registry, handle) = fixture("stop_ends", &upstream).await;
    registry
        .connect(&handle, None, true)
        .await
        .expect("connect");
    upstream.wait_opened(0).await;

    registry.disconnect(&handle).await;

    assert!(!handle.is_running().await, "stop left the account running");
    assert_eq!(
        registry.connected_count(),
        0,
        "a stopped bot still holds its connection budget slot"
    );
    upstream.assert_closed(0).await;

    registry.delete(&handle).await.expect("delete the account");
}

#[tokio::test]
async fn a_connect_after_a_timed_out_stop_runs_alone() {
    let mut upstream = SilentUpstream::start().await;
    let (registry, handle) = fixture("connect_after", &upstream).await;
    registry
        .connect(&handle, None, true)
        .await
        .expect("first connect");
    upstream.wait_opened(0).await;
    registry.disconnect(&handle).await;

    registry
        .connect(&handle, None, true)
        .await
        .expect("second connect");
    upstream.wait_opened(1).await;
    assert_eq!(
        registry.connected_count(),
        1,
        "the stopped bot is still counted next to the new one"
    );

    registry.disconnect(&handle).await;

    assert!(!handle.is_running().await, "stop left the account running");
    assert_eq!(
        registry.connected_count(),
        0,
        "the second bot still holds a budget slot after its stop"
    );
    upstream.assert_closed(0).await;
    upstream.assert_closed(1).await;

    registry.delete(&handle).await.expect("delete the account");
}
