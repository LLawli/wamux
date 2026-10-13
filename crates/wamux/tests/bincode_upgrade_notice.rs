//! #84: 0.2.x is the last release line that converts a bincode store; 0.3.0
//! deletes the conversion. What the operator is told: a WARN when a store is
//! converted, and the store's `blob_format` in `AdminService.Check`, read from
//! the marker on every call.
//!
//! Its own binary, because the capture is a process-wide subscriber: other
//! suites' events stay out of it. Every test here starts with
//! `capture_warnings()`, see there for why.

// Only a subset of the shared helpers is used per test binary.
#[allow(dead_code)]
mod common;
#[allow(dead_code)]
mod legacy_store;

use std::cell::RefCell;
use std::io::Write;
use std::sync::{Arc, Once};

use legacy_store::{legacy_account, legacy_sqlite, raw_sqlite};
use wamux::proto::v1 as pb;
use wamux::proto::v1::admin_service_client::AdminServiceClient;
use wamux::state::{AccountRegistry, RegistryTuning};
use wamux::storage::StorageEngine;
use wamux::storage::StoreBlobFormat;
use wamux::storage::sql::SqlStore;

thread_local! {
    /// What the subscriber wrote on this thread. `#[tokio::test]` runs the test
    /// on its own thread, so each test reads only its own events.
    static CAPTURED: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// The subscriber's writer: appends to the calling thread's `CAPTURED`.
struct ThreadLog;

impl Write for ThreadLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        CAPTURED.with(|captured| captured.borrow_mut().extend_from_slice(buf));
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// This thread's capture.
struct CapturedLog;

impl CapturedLog {
    /// The WARN lines captured on this thread since the last call, drained.
    fn take_warnings(&self) -> Vec<String> {
        let bytes = CAPTURED.with(|captured| std::mem::take(&mut *captured.borrow_mut()));
        String::from_utf8(bytes)
            .unwrap()
            .lines()
            .filter(|line| line.contains("WARN"))
            .map(str::to_string)
            .collect()
    }
}

static INSTALL_SUBSCRIBER: Once = Once::new();

/// Install the WARN-and-above subscriber (once per process) and start this
/// thread's capture empty. Global, not `set_default`: a thread-local default
/// raced with the callsite interest cache when another test thread hit the
/// same `warn!` with no subscriber, and the callsite stayed off (1 run in
/// 200, seen on hosted CI, #84). It also has to come first in EVERY test, so
/// no event of this binary is emitted before the subscriber exists.
fn capture_warnings() -> CapturedLog {
    INSTALL_SUBSCRIBER.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .with_writer(|| ThreadLog)
            .finish();
        tracing::subscriber::set_global_default(subscriber).expect("the only subscriber");
    });
    CAPTURED.with(|captured| captured.borrow_mut().clear());
    CapturedLog
}

/// The one line the conversion owes the operator: what 0.3.0 drops, and how
/// to get past it.
fn assert_the_upgrade_path_is_announced(warnings: &[String]) {
    let announced = warnings
        .iter()
        .filter(|line| line.contains("0.3.0") && line.contains("0.2.x"))
        .count();
    assert_eq!(
        announced, 1,
        "one WARN naming 0.3.0 and 0.2.x, got: {warnings:#?}"
    );
}

/// No WARN about bincode or 0.3.0. Other warnings, like the #75 file mode
/// one, are not these tests' business.
fn assert_nothing_about_bincode(warnings: Vec<String>) {
    let about_bincode: Vec<String> = warnings
        .into_iter()
        .filter(|line| line.contains("bincode") || line.contains("0.3.0"))
        .collect();
    assert!(
        about_bincode.is_empty(),
        "nothing to announce: {about_bincode:#?}"
    );
}

// --- the warning ---

#[tokio::test]
async fn sqlite_conversion_warns_that_0_3_0_drops_bincode() {
    let log = capture_warnings();
    let (url, _dir, _device_id) = legacy_sqlite(&legacy_account(), false).await;

    SqlStore::open_sqlite(&url).await.expect("open converts");

    assert_the_upgrade_path_is_announced(&log.take_warnings());
}

/// The warning is about the conversion, not about every start: the first
/// open converts and announces, the second finds 'protobuf' and says nothing
/// about bincode.
#[tokio::test]
async fn sqlite_protobuf_store_opens_without_the_warning() {
    let log = capture_warnings();
    let (url, _dir, _device_id) = legacy_sqlite(&legacy_account(), false).await;
    let first = SqlStore::open_sqlite(&url).await.expect("open converts");
    assert_the_upgrade_path_is_announced(&log.take_warnings());
    first.close().await.expect("close the converted store");

    SqlStore::open_sqlite(&url).await.expect("reopen");

    assert_nothing_about_bincode(log.take_warnings());
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_conversion_warns_that_0_3_0_drops_bincode() {
    let log = capture_warnings();
    let (_url, dir, _device_id) = legacy_sqlite(&legacy_account(), false).await;
    let turso_url = format!("turso://{}", dir.path().join("legacy.db").display());

    wamux::storage::turso::TursoStore::open(&turso_url)
        .await
        .expect("open converts");

    assert_the_upgrade_path_is_announced(&log.take_warnings());
}

/// A brand-new store is created with the marker at 'bincode' (migration
/// 0003/0004) and "converts" zero rows on its first open. There is nothing in it
/// that 0.3.0 could not read, so the operator is told nothing (found live, #84).
#[tokio::test]
async fn sqlite_fresh_store_opens_without_the_warning() {
    let log = capture_warnings();
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("fresh.db").display()
    );

    SqlStore::open_sqlite(&url)
        .await
        .expect("open a fresh store");

    assert_nothing_about_bincode(log.take_warnings());
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_fresh_store_opens_without_the_warning() {
    let log = capture_warnings();
    let dir = tempfile::tempdir().unwrap();
    let url = format!("turso://{}", dir.path().join("fresh.db").display());

    wamux::storage::turso::TursoStore::open(&url)
        .await
        .expect("open a fresh store");

    assert_nothing_about_bincode(log.take_warnings());
}

// --- the health report ---

#[tokio::test]
async fn sqlite_health_reports_protobuf_after_converting_a_legacy_store() {
    let _log = capture_warnings(); // first, see capture_warnings
    let (url, _dir, _device_id) = legacy_sqlite(&legacy_account(), false).await;
    let engine = Arc::new(SqlStore::open_sqlite(&url).await.expect("open converts"));
    let registry = Arc::new(AccountRegistry::new(engine, RegistryTuning::with_ring(64)));
    let mut admin = AdminServiceClient::new(common::serve_registry(registry).await);

    let health = admin.check(pb::Empty {}).await.expect("check").into_inner();

    assert!(health.ready);
    assert_eq!(
        pb::BlobFormat::try_from(health.blob_format),
        Ok(pb::BlobFormat::Protobuf)
    );
}

// --- the marker, read live ---

#[tokio::test]
async fn sqlite_blob_format_reads_the_marker_live() {
    let _log = capture_warnings(); // first, see capture_warnings
    let (engine, dir) = common::sqlite_engine().await;
    assert_eq!(
        engine.blob_format().await.unwrap(),
        StoreBlobFormat::Protobuf
    );

    let url = format!("sqlite://{}", dir.path().join("wamux-test.db").display());
    let writer = raw_sqlite(&url).await;
    let mark = |format: &'static str| {
        sqlx::query("UPDATE blob_format SET format = ? WHERE id = 1").bind(format)
    };
    mark("bincode").execute(&writer).await.unwrap();
    assert_eq!(
        engine.blob_format().await.unwrap(),
        StoreBlobFormat::Bincode
    );

    mark("json").execute(&writer).await.unwrap();
    let unknown = engine.blob_format().await;
    assert!(
        unknown.is_err(),
        "an unknown marker is an error: {unknown:?}"
    );
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_blob_format_reads_the_marker_live() {
    let _log = capture_warnings(); // first, see capture_warnings
    let (engine, _dir) = common::turso_engine().await;
    assert_eq!(
        engine.blob_format().await.unwrap(),
        StoreBlobFormat::Protobuf
    );

    let mark = |format: &'static str| {
        let sql = "UPDATE blob_format SET format = ?1 WHERE id = 1";
        common::turso_rows(&engine, sql, vec![turso::Value::Text(format.into())])
    };
    mark("bincode").await;
    assert_eq!(
        engine.blob_format().await.unwrap(),
        StoreBlobFormat::Bincode
    );

    mark("json").await;
    let unknown = engine.blob_format().await;
    assert!(
        unknown.is_err(),
        "an unknown marker is an error: {unknown:?}"
    );
}
