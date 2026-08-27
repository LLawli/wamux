//! Shared helpers for the integration-test binaries (each test file compiles
//! as its own crate and pulls this in via `mod common;`).

use std::sync::Arc;

use wamux::proto::v1 as pb;
use wamux::storage::StorageEngine;
use wamux::storage::postgres::PgStorage;
use wamux::storage::sqlite::SqliteStorage;

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
    let requested = std::env::var("WAMUX_TEST_ENGINE").unwrap_or_else(|_| "postgres".into());
    match requested.as_str() {
        "postgres" => pg_engine(5).await,
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
