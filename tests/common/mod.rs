//! Shared helpers for the integration-test binaries (each test file compiles
//! as its own crate and pulls this in via `mod common;`).

/// Delete leftover synthetic accounts whose `external_ref` starts with
/// `prefix`. Tests call this at setup (self-heal from an aborted prior run,
/// whose best-effort teardown never ran) and at the end, bounding accumulation
/// to at most one aborted run's rows (the B5 pattern, docs/BACKLOG.md).
pub async fn sweep_orphans(pool: &sqlx::PgPool, prefix: &str) -> u64 {
    sqlx::query("DELETE FROM accounts WHERE external_ref LIKE $1")
        .bind(format!("{prefix}%"))
        .execute(pool)
        .await
        .map(|r| r.rows_affected())
        .unwrap_or(0)
}
