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
use wamux::storage::sql::{SqlPool, SqlStore};
use wamux::{server, transport};

// `MockWaServer` exists only in a stress build.
#[cfg(feature = "stress")]
pub mod mock_wire;

/// `WAMUX_TEST_ENCRYPT=1` opens the file-backed engines (SQLite, Turso) with a
/// store key, so a whole suite runs against sealed blobs (#164). Postgres is
/// not keyed here: the shared test database would turn `encrypted` for good and
/// refuse every later keyless run; its keyed case has a throwaway database.
pub fn encrypted_tests() -> bool {
    std::env::var("WAMUX_TEST_ENCRYPT").is_ok_and(|value| value == "1")
}

/// The key `encrypted_tests` opens with; `None` when the suite runs plaintext.
pub fn test_store_key() -> Option<wamux::storage::StoreKey> {
    // expect: a literal of 64 hex characters.
    encrypted_tests().then(|| wamux::storage::StoreKey::parse_hex(&"ab".repeat(32)).expect("hex"))
}

/// The dockerized test database (CLAUDE.md's wamux-pg on :5433) unless the
/// environment points elsewhere — the single home of the default DSN.
pub fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wamux:wamux@localhost:5433/wamux".into())
}

/// Connect + migrate the Postgres engine in one call; `max_conns` is the only
/// knob the suites vary.
pub async fn pg_engine(max_conns: u32) -> Arc<SqlStore> {
    Arc::new(
        SqlStore::open_postgres(&database_url(), max_conns)
            .await
            .expect("open pg storage"),
    )
}

/// The Postgres pool under a `SqlStore` a test opened as Postgres, for the
/// probes that read below the store traits.
pub fn pg_pool(store: &SqlStore) -> &sqlx::PgPool {
    match store.pool() {
        SqlPool::Pg(pool) => pool,
        SqlPool::Sqlite(_) => panic!("opened as postgres, holds a sqlite pool"),
    }
}

/// The SQLite pool under a `SqlStore` a test opened as SQLite.
pub fn lite_pool(store: &SqlStore) -> &sqlx::SqlitePool {
    match store.pool() {
        SqlPool::Sqlite(pool) => pool,
        SqlPool::Pg(_) => panic!("opened as sqlite, holds a postgres pool"),
    }
}

/// A fresh SQLite engine in a throwaway directory, plus the guard that keeps
/// the directory alive. Every call gets its own empty database file.
pub async fn sqlite_engine() -> (Arc<SqlStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("wamux-test.db");
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let engine = SqlStore::open_sqlite_keyed(&url, test_store_key().as_ref())
        .await
        .expect("open sqlite storage");
    (Arc::new(engine), dir)
}

/// A fresh Turso engine in a throwaway directory (#106), like `sqlite_engine`.
#[cfg(feature = "turso")]
pub async fn turso_engine() -> (Arc<wamux::storage::turso::TursoStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let url = format!("turso://{}", dir.path().join("wamux-test.db").display());
    let engine = wamux::storage::turso::TursoStore::open_keyed(&url, test_store_key().as_ref())
        .await
        .expect("open turso storage");
    (Arc::new(engine), dir)
}

/// Every row of a raw statement on a Turso store, each as its column values.
/// Raw SQL in tests writes `?N`: the `$N` rewrite is the engine's, not theirs.
#[cfg(feature = "turso")]
pub async fn turso_rows(
    store: &wamux::storage::turso::TursoStore,
    sql: &str,
    params: Vec<turso::Value>,
) -> Vec<Vec<turso::Value>> {
    let conn = store.connection().lock().await;
    let mut rows = conn
        .query(sql, params)
        .await
        .unwrap_or_else(|e| panic!("turso query {sql:?}: {e}"));
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .unwrap_or_else(|e| panic!("turso step {sql:?}: {e}"))
    {
        let values = (0..row.column_count())
            .map(|i| row.get_value(i).expect("turso column"))
            .collect();
        out.push(values);
    }
    out
}

/// The engine under test, selected by `WAMUX_TEST_ENGINE` (`postgres` — the
/// default — `sqlite`, or `turso` in a `--features turso` build). This is what lets scripts/ci.sh run the whole suite
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
        #[cfg(feature = "turso")]
        "turso" => {
            let (engine, dir) = turso_engine().await;
            Box::leak(Box::new(dir));
            engine
        }
        other => panic!(
            "unknown WAMUX_TEST_ENGINE '{other}' (expected postgres, sqlite, or turso \
             in a --features turso build)"
        ),
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
    registered_account_as(ws_url, prefix, "5511999999999@s.whatsapp.net", |_| {}).await
}

/// `registered_account` with the device's own `pn` (a companion carries its
/// device id, `user:7@s.whatsapp.net`) and a last touch on the `Device` before
/// it is saved (#71).
pub async fn registered_account_as(
    ws_url: String,
    prefix: &str,
    pn: &str,
    shape: impl FnOnce(&mut Device),
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
    device.pn = Some(pn.parse().expect("parse pn jid"));
    device.push_name = "Stress".to_string();
    shape(&mut device);
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
    log_in(registry, handle).await
}

/// The companion device the MessagingService suite logs in as (#71): device 7
/// of `5511999999999`, whose phone (device 0) the suite serves as a
/// `MockPeer`. A peer message (history on demand) goes to that phone, and
/// every DM fans out to it, as it does for a real linked device.
#[cfg(feature = "stress")]
pub const COMPANION_PN: &str = "5511999999999:7@s.whatsapp.net";

/// Log in as `COMPANION_PN` against a mock started with `start_as`, with what a
/// paired companion holds that the plain test device does not: an ADV account
/// identity (a zero-filled one, as the library's own CallFixture seeds: the
/// client embeds it in a pkmsg and never verifies it) and app-state sync
/// state, so a chat action sends its patch at once. Waits until the client
/// has also taken its LID from `<success>`, which group and status sends need.
#[cfg(feature = "stress")]
pub async fn logged_in_companion(mock: &wamux::stress::MockWaServer, prefix: &str) -> LoggedIn {
    let (registry, handle) = registered_account_as(mock.ws_url(), prefix, COMPANION_PN, |device| {
        device.account = Some(Arc::new(zeroed_adv_identity()));
    })
    .await;
    let backend = registry.storage().device_backend(handle.device_id);
    wamux::stress::app_state_fixture::seed_app_state(backend.as_ref())
        .await
        .expect("seed app state");
    let logged = log_in(registry, handle).await;
    let client = logged.client.clone();
    poll_until(
        "the client to take its LID",
        Duration::from_secs(10),
        || async { client.lid().map(drop) },
    )
    .await;
    logged
}

#[cfg(feature = "stress")]
fn zeroed_adv_identity() -> whatsapp_rust::waproto::whatsapp::ADVSignedDeviceIdentity {
    whatsapp_rust::waproto::whatsapp::ADVSignedDeviceIdentity {
        details: Some(vec![0; 32]),
        account_signature_key: Some(vec![0; 32]),
        account_signature: Some(vec![0; 64]),
        device_signature: Some(vec![0; 64]),
    }
}

/// Connect a registered account and wait (bounded) until it is logged in.
#[cfg(feature = "stress")]
async fn log_in(registry: Arc<AccountRegistry>, handle: Arc<AccountHandle>) -> LoggedIn {
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
    serve_registry_with(registry, Config::default()).await
}

/// `serve_registry` with a config of the test's choosing (#71: a small
/// `media_max_bytes`). The socket path and reflection are always the test's.
pub async fn serve_registry_with(registry: Arc<AccountRegistry>, config: Config) -> Channel {
    // Leaked on purpose, like the engine dir above: the socket has to outlive
    // the server task and the process is about to exit anyway.
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tempdir")));
    let socket = dir.path().join("wamux.sock");
    let socket_str = socket.to_str().expect("utf-8 socket path").to_string();
    let config = Config {
        socket_path: socket_str.clone(),
        enable_reflection: false,
        ..config
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
