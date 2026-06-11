//! Shared helpers for the integration-test binaries (each test file compiles
//! as its own crate and pulls this in via `mod common;`).

use wamux::proto::v1 as pb;

/// The dockerized test database (CLAUDE.md's wamux-pg on :5433) unless the
/// environment points elsewhere — the single home of the default DSN.
pub fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wamux:wamux@localhost:5433/wamux".into())
}

/// Connect + migrate in one call; `max_conns` is the only knob the suites vary.
pub async fn connect_and_migrate(max_conns: u32) -> sqlx::PgPool {
    let pool = wamux::storage::postgres::connect(&database_url(), max_conns)
        .await
        .expect("connect pg");
    wamux::storage::postgres::run_migrations(&pool)
        .await
        .expect("migrate");
    pool
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
pub async fn sweep_orphans(pool: &sqlx::PgPool, prefix: &str) -> u64 {
    // LIKE-escape `\`, `%`, `_` so a prefix like "evsub_" matches literally
    // instead of as a single-char wildcard (Postgres default escape is `\`).
    let escaped = prefix
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    sqlx::query("DELETE FROM accounts WHERE external_ref LIKE $1")
        .bind(format!("{escaped}%"))
        .execute(pool)
        .await
        .map(|r| r.rows_affected())
        .unwrap_or(0)
}
