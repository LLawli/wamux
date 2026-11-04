//! #247: a pairing stream ends with the daemon and with its client.
//!
//! `PairWithQr` and `PairWithCode` wait on a phone, so left alone their streams
//! hold a connection open: every shutdown during a pairing cost the whole
//! `shutdown_grace_ms`, and the client saw a torn connection. A forwarder that
//! outlives its client must not keep the account alive either. What ends a
//! forwarder, cause by cause, is in `account_service_tests.rs`.
//!
//! `connect` always runs under these RPCs, so each test picks its upstream (see
//! `Upstream`). The `stress` build pins the version, so nothing reaches the real
//! `sw.js`. The QR is published through the account's own publisher, the way
//! the library's event arrives, so each test starts from a stream proven live.
//! Honors `WAMUX_TEST_ENGINE`.

use crate::common;
use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;
use tokio_stream::StreamExt;
use tonic::transport::Channel;
use wacore::types::events::{Event, PairingQrCode};
use wamux::config::Config;
use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::server::{self, Drained};
use wamux::state::{AccountHandle, AccountRegistry, RegistryTuning};
use wamux::stress::MockWaServer;
use wamux::transport::{self, shutdown::Shutdown};

use common::mock_wire::account_ref;

/// Far above `DRAIN`: passing on the backstop would hide a regression in the
/// clean path, so the tests demand `Drained::Clean`.
const GRACE: Duration = Duration::from_secs(30);
const DRAIN: Duration = Duration::from_secs(5);
/// Long enough for a loaded CI runner, short enough to fail fast.
const EVENT_WAIT: Duration = Duration::from_secs(10);
const QR: &str = "2@pairing-shutdown-qr";

type Pairing = tonic::Streaming<pb::PairingUpdate>;
type Serving = JoinHandle<Result<Drained, tonic::transport::Error>>;

/// What the account's connect talks to.
enum Upstream {
    /// Accepts and never answers: the connect sits in its handshake, so the
    /// stream stays open. The mock would end a PairWithCode stream, because a
    /// pair code request it cannot answer is a terminal `code_error`.
    Silent(TcpListener),
    /// A real handshake, so `stop` ends the bot. Stuck on a silent upstream it
    /// outlives `stop`, and its supervisor keeps the account alive.
    Mock(MockWaServer),
}

impl Upstream {
    fn silent() -> Self {
        Self::Silent(TcpListener::bind("127.0.0.1:0").expect("bind the silent upstream"))
    }

    async fn mock() -> Self {
        Self::Mock(MockWaServer::start().await.expect("start mock"))
    }

    fn ws_url(&self) -> String {
        match self {
            Self::Silent(listener) => {
                let port = listener.local_addr().expect("upstream address").port();
                format!("ws://127.0.0.1:{port}")
            }
            Self::Mock(mock) => mock.ws_url(),
        }
    }
}

struct Fixture {
    _upstream: Upstream,
    _dir: tempfile::TempDir,
    registry: Arc<AccountRegistry>,
    handle: Arc<AccountHandle>,
    shutdown: Shutdown,
    server: Serving,
    accounts: AccountServiceClient<Channel>,
}

async fn fixture(test: &str, upstream: Upstream) -> Fixture {
    let engine = common::test_engine().await;
    let prefix = common::test_prefix("pairing_shutdown", test);
    common::sweep_orphans(&engine, &prefix).await;
    let tuning = RegistryTuning {
        ws_url_override: Some(upstream.ws_url()),
        ..RegistryTuning::default()
    };
    let registry = Arc::new(AccountRegistry::new(engine, tuning));
    let handle = registry
        .create_account(Some(&format!("{prefix}{}", uuid::Uuid::new_v4())))
        .await
        .expect("create account");

    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("wamux.sock");
    let socket_str = socket.to_str().unwrap().to_string();
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
    let accounts = AccountServiceClient::new(common::uds_channel(&socket).await);
    Fixture {
        _upstream: upstream,
        _dir: dir,
        registry,
        handle,
        shutdown,
        server,
        accounts,
    }
}

impl Fixture {
    fn account(&self) -> Option<pb::AccountRef> {
        Some(account_ref(&self.handle.uuid.to_string()))
    }

    async fn pair_with_qr(&mut self) -> Pairing {
        let request = pb::PairWithQrRequest {
            account: self.account(),
            backfill_history: false,
        };
        let stream = self.accounts.pair_with_qr(request).await;
        stream.expect("PairWithQr").into_inner()
    }

    async fn pair_with_code(&mut self) -> Pairing {
        let request = pb::PairWithCodeRequest {
            account: self.account(),
            phone_number: "5511999999999".to_string(),
            custom_code: None,
            backfill_history: false,
        };
        let stream = self.accounts.pair_with_code(request).await;
        stream.expect("PairWithCode").into_inner()
    }

    /// `connect` runs beside the stream. A delete that lands before it has
    /// started the bot stops nothing, and the bot then holds the account.
    async fn wait_until_connecting(&self) {
        let deadline = Instant::now() + EVENT_WAIT;
        while !self.handle.is_running().await {
            assert!(Instant::now() < deadline, "the connect never started");
            // not a sync point: the connect runs in a task of its own and announces nothing a test can await
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// The stream is live: a QR published on the account reaches the client.
    async fn see_a_qr_on(&self, stream: &mut Pairing) {
        self.handle
            .publisher
            .publish_lib(Arc::new(Event::PairingQrCode(
                PairingQrCode::builder()
                    .code(QR.to_string())
                    .timeout(Duration::from_secs(60))
                    .build(),
            )));
        let update = tokio::time::timeout(EVENT_WAIT, stream.next()).await;
        let update = update.expect("no QR within the wait");
        let update = update.expect("the stream ended before the QR");
        assert_eq!(
            update.expect("pairing stream").event,
            Some(pb::pairing_update::Event::QrCode(QR.to_string()))
        );
    }

    /// Fires shutdown with `stream` open, and proves it ended the stream.
    async fn assert_shutdown_ends(self, mut stream: Pairing) {
        self.shutdown.trigger();

        let finished = tokio::time::timeout(DRAIN, self.server).await;
        let drained = finished
            .expect("the server was still serving after shutdown: a pairing stream pins it")
            .expect("server task panicked")
            .expect("serve failed");
        assert_eq!(
            drained,
            Drained::Clean,
            "the pairing stream must end on its own, not be cut off by the grace"
        );

        // A clean end of stream, not a torn connection, so an edge can tell
        // "the daemon is restarting" from "the network broke".
        let end = tokio::time::timeout(Duration::from_secs(1), stream.next()).await;
        assert!(
            matches!(end, Ok(None)),
            "expected a clean end of stream, got {end:?}"
        );

        self.registry
            .delete(&self.handle)
            .await
            .expect("delete the account");
    }
}

#[tokio::test]
async fn shutdown_ends_an_open_pair_with_qr_stream() {
    let mut fixture = fixture("shutdown_ends_qr", Upstream::silent()).await;
    let mut stream = fixture.pair_with_qr().await;
    fixture.see_a_qr_on(&mut stream).await;
    fixture.assert_shutdown_ends(stream).await;
}

#[tokio::test]
async fn shutdown_ends_an_open_pair_with_code_stream() {
    let mut fixture = fixture("shutdown_ends_code", Upstream::silent()).await;
    let mut stream = fixture.pair_with_code().await;
    fixture.see_a_qr_on(&mut stream).await;
    fixture.assert_shutdown_ends(stream).await;
}

/// Nothing else holds the handle once the account is deleted, so a handle that
/// stays alive is a pairing task still holding it. This does not prove the
/// forwarder ended: it no longer owns the handle, so a parked one passes too.
#[tokio::test]
async fn a_client_hanging_up_leaves_nothing_holding_the_account() {
    let mut fixture = fixture("hang_up", Upstream::mock().await).await;
    let mut stream = fixture.pair_with_qr().await;
    fixture.see_a_qr_on(&mut stream).await;
    fixture.wait_until_connecting().await;
    let released = Arc::downgrade(&fixture.handle);

    drop(stream);
    fixture
        .registry
        .delete(&fixture.handle)
        .await
        .expect("delete the account");
    let Fixture {
        handle, shutdown, ..
    } = fixture;
    drop(handle);

    let deadline = Instant::now() + EVENT_WAIT;
    while released.strong_count() > 0 {
        assert!(
            Instant::now() < deadline,
            "something still holds the account after its client hung up"
        );
        // not a sync point: the task ending is the signal, and nothing observable fires when an Arc is released
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    shutdown.trigger();
}
