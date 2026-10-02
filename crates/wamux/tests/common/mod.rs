//! Shared helpers for the integration-test binaries (each test file compiles
//! as its own crate and pulls this in via `mod common;`).

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use hyper_util::rt::TokioIo;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;
use wacore::store::Device;
use wamux::config::Config;
use wamux::proto::v1 as pb;
use wamux::state::{AccountHandle, AccountRegistry, RegistryTuning};
use wamux::storage::StorageEngine;
use wamux::storage::postgres::PgStorage;
use wamux::storage::sqlite::SqliteStorage;
use wamux::{server, transport};

// `MockWaServer` exists only in a stress build.
#[cfg(feature = "stress")]
pub mod mock_wire;

/// The dockerized test database (CLAUDE.md's wamux-pg on :5433) unless the
/// environment points elsewhere — the single home of the default DSN.
pub fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wamux:wamux@localhost:5433/wamux".into())
}

/// Connect + migrate the Postgres engine in one call; `max_conns` is the only
/// knob the suites vary.
pub async fn pg_engine(max_conns: u32) -> Arc<PgStorage> {
    Arc::new(
        PgStorage::open(&database_url(), max_conns)
            .await
            .expect("open pg storage"),
    )
}

/// A fresh SQLite engine in a throwaway directory, plus the guard that keeps
/// the directory alive. Every call gets its own empty database file.
pub async fn sqlite_engine() -> (Arc<SqliteStorage>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("wamux-test.db");
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let engine = SqliteStorage::open(&url)
        .await
        .expect("open sqlite storage");
    (Arc::new(engine), dir)
}

/// The engine under test, selected by `WAMUX_TEST_ENGINE` (`postgres` — the
/// default — or `sqlite`). This is what lets scripts/ci.sh run the whole suite
/// twice, once per engine, and the SQLite pass needs no Postgres container.
///
/// The SQLite temp dir is leaked on purpose, matching how these tests already
/// handle the socket dir: the database has to outlive the server task that
/// keeps using it, and the process is about to exit anyway.
pub async fn test_engine() -> Arc<dyn StorageEngine> {
    test_engine_with(5).await
}

/// `test_engine()` with a pool size: the M3 scale test needs more than 5 (#67).
/// SQLite ignores `max_conns`.
pub async fn test_engine_with(max_conns: u32) -> Arc<dyn StorageEngine> {
    let requested = std::env::var("WAMUX_TEST_ENGINE").unwrap_or_else(|_| "postgres".into());
    match requested.as_str() {
        "postgres" => pg_engine(max_conns).await,
        "sqlite" => {
            let (engine, dir) = sqlite_engine().await;
            Box::leak(Box::new(dir));
            engine
        }
        other => panic!("unknown WAMUX_TEST_ENGINE '{other}' (expected postgres or sqlite)"),
    }
}

/// Synthetic raw envelope for driving the event fan-out without a real
/// WhatsApp connection. `payload_len` > 0 adds flood weight for load tests.
pub fn synthetic_envelope(account_uuid: &str, seq: i64, payload_len: usize) -> pb::EventEnvelope {
    pb::EventEnvelope {
        account_uuid: account_uuid.to_string(),
        monotonic_seq: seq,
        ts_unix_ms: 0,
        event: Some(pb::event_envelope::Event::Raw(pb::RawEvent {
            kind: "synthetic".to_string(),
            payload: vec![0u8; payload_len],
            note: String::new(),
        })),
    }
}

/// Delete leftover synthetic accounts whose `external_ref` starts with
/// `prefix`. Tests call this at setup (self-heal from an aborted prior run,
/// whose best-effort teardown never ran) and at the end, bounding accumulation
/// to at most one aborted run's rows (the B5 pattern, docs/BACKLOG.md).
///
/// Prefix matching happens in Rust, over `list_accounts`, rather than in SQL:
/// that keeps the helper engine-agnostic and sidesteps per-dialect LIKE escaping
/// (the old Postgres version had to escape `\`, `%` and `_` by hand). Test-only,
/// so the full-table scan is irrelevant.
pub async fn sweep_orphans(storage: &Arc<dyn StorageEngine>, prefix: &str) -> u64 {
    let rows = match storage.list_accounts().await {
        Ok(rows) => rows,
        Err(_) => return 0,
    };
    let mut deleted = 0;
    for row in rows {
        let matches = row
            .external_ref
            .as_deref()
            .is_some_and(|external| external.starts_with(prefix));
        if matches && storage.delete_account(row.uuid).await.unwrap_or(false) {
            deleted += 1;
        }
    }
    deleted
}

/// Build a registry pointed at a mock WhatsApp endpoint (`ws_url`) and create
/// an account (`external_ref` = `prefix` + a fresh uuid, after sweeping `prefix`
/// of leftovers; `prefix` comes from `test_prefix`, #67) whose device is
/// already *registered* (pn set + persisted), so
/// `connect` makes the client send a LOGIN payload and treat the mock's
/// `<success>` as auth success. Returns the registry and the (not-yet-connected)
/// account handle. Shared by the stress suites.
pub async fn registered_account(
    ws_url: String,
    prefix: &str,
) -> (Arc<AccountRegistry>, Arc<AccountHandle>) {
    let engine = test_engine().await;
    sweep_orphans(&engine, prefix).await;
    let tuning = RegistryTuning {
        ws_url_override: Some(ws_url),
        ..RegistryTuning::default()
    };
    let registry = Arc::new(AccountRegistry::new(engine, tuning));

    let tag = uuid::Uuid::new_v4();
    let handle = registry
        .create_account(Some(&format!("{prefix}{tag}")))
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

/// The external_ref prefix of one test's accounts: `<suite>/<test>/` (#67).
/// The trailing `/` is what keeps one prefix from matching another: with `-`,
/// sweeping `stress-nl-` also swept `stress-nl-hist-` (the bug #60 hit). Panics
/// if either part contains a `/`.
pub fn test_prefix(suite: &str, test: &str) -> String {
    assert!(
        !suite.contains('/') && !test.contains('/'),
        "test_prefix parts must not contain '/': suite={suite:?} test={test:?}"
    );
    format!("{suite}/{test}/")
}

/// Pause between probes of `poll_until`.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// The one bounded wait on a condition (#67): calls `probe` until it returns
/// `Some`, and panics naming `what` once `timeout` has passed. Every retry loop
/// in the suites goes through here, so no test synchronizes on a fixed sleep.
pub async fn poll_until<T, F, Fut>(what: &str, timeout: Duration, mut probe: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(found) = probe().await {
            return found;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out after {timeout:?} waiting for {what}"
        );
        // not a sync point: the pause between two probes of a bounded wait; the condition is what is awaited
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Wait (bounded) until the account's broadcast has a live receiver: forwarder
/// attachment is async, and a `broadcast::send` with zero receivers is lost,
/// so pushing before attachment would race (#67).
pub async fn await_forwarder_attached(
    events_tx: &tokio::sync::broadcast::Sender<pb::EventEnvelope>,
    which: &str,
) {
    poll_until(
        &format!("a forwarder attached to the {which} account"),
        Duration::from_secs(5),
        || async { (events_tx.receiver_count() > 0).then_some(()) },
    )
    .await;
}

/// A tonic channel over the daemon's Unix socket, retrying (bounded) while the
/// server task binds it. The one connector the socket suites share (#67).
pub async fn uds_channel(path: &Path) -> Channel {
    poll_until(
        "the server to accept a connection",
        Duration::from_secs(5),
        || connect_uds_once(path.to_path_buf()),
    )
    .await
}

async fn connect_uds_once(path: PathBuf) -> Option<Channel> {
    Endpoint::try_from("http://[::1]:50051")
        .expect("static endpoint uri")
        .connect_with_connector(service_fn(move |_: Uri| {
            let path = path.clone();
            async move {
                let stream = tokio::net::UnixStream::connect(path).await?;
                Ok::<_, std::io::Error>(TokioIo::new(stream))
            }
        }))
        .await
        .ok()
}

/// A client logged in against the mock, and what it takes to clean it up.
#[cfg(feature = "stress")]
pub struct LoggedIn {
    pub registry: Arc<AccountRegistry>,
    pub handle: Arc<AccountHandle>,
    pub client: Arc<whatsapp_rust::Client>,
}

#[cfg(feature = "stress")]
impl LoggedIn {
    /// Disconnect and delete the account, so the run leaves no row behind.
    pub async fn cleanup(self) {
        self.registry.disconnect(&self.handle).await;
        self.registry
            .delete(&self.handle)
            .await
            .expect("delete the test account");
    }
}

/// Register an account under `prefix`, connect it to the mock and wait until
/// it is logged in. Not `wait_for_connected`: its Ready stage waits on
/// post-login syncs the mock does not serve, and logged in is all these
/// suites need. The one definition the stress suites share (#67).
#[cfg(feature = "stress")]
pub async fn logged_in_client(mock: &wamux::stress::MockWaServer, prefix: &str) -> LoggedIn {
    let (registry, handle) = registered_account(mock.ws_url(), prefix).await;
    registry
        .connect(&handle, None, true)
        .await
        .expect("connect");
    let client = handle.client().await.expect("client after connect");
    poll_until("the client to log in", Duration::from_secs(10), || async {
        client.is_logged_in().then_some(())
    })
    .await;
    LoggedIn {
        registry,
        handle,
        client,
    }
}

/// Serve `registry` on a throwaway socket and return a channel to it (#68).
/// The mock-backed service suites build the registry with
/// `registered_account`/`logged_in_client`, so the account the RPCs name is
/// the one connected to the mock. `spawn_server` (moved here from
/// `grpc_server.rs`) is this over a fresh `test_engine()` registry.
pub async fn serve_registry(registry: Arc<AccountRegistry>) -> Channel {
    // Leaked on purpose, like the engine dir above: the socket has to outlive
    // the server task and the process is about to exit anyway.
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tempdir")));
    let socket = dir.path().join("wamux.sock");
    let socket_str = socket.to_str().expect("utf-8 socket path").to_string();
    let config = Config {
        socket_path: socket_str.clone(),
        enable_reflection: false,
        ..Config::default()
    };
    let stream = transport::uds_listener::bind(&socket_str, 0o660, None).expect("bind");
    let router = server::build_router(registry, &config, transport::shutdown::Shutdown::new());
    tokio::spawn(async move {
        let _ = router.serve_with_incoming(stream).await;
    });
    uds_channel(&socket).await
}

/// Spin the server on a throwaway socket and return a connected channel plus
/// the engine behind it (the LID-mapping suite writes through the same storage
/// the RPC reads). Moved here from `grpc_server.rs` in #68.
pub async fn spawn_server() -> (Channel, Arc<dyn StorageEngine>) {
    let engine = test_engine().await;
    let registry = Arc::new(AccountRegistry::new(
        engine.clone(),
        RegistryTuning::with_ring(64),
    ));
    (serve_registry(registry).await, engine)
}
