//! No test run may leave rows behind in the shared Postgres (#67).
//!
//! The suites create accounts (and the bincode suite whole `wamux_upgrade_*`
//! databases) in the dockerized test database, and for a long time several
//! never deleted them: 176 accounts had piled up by the time #67 counted.
//! `scripts/ci.sh` runs these two around the whole Postgres-backed suite:
//!
//! 1. `snapshot_accounts_before_the_run` writes every account uuid and every
//!    throwaway database name to `$WAMUX_ACCOUNT_SNAPSHOT`;
//! 2. `no_account_outlives_the_run` fails if anything exists now that did not
//!    exist then.
//!
//! A snapshot rather than "the table is empty": the database is also the dev
//! one, so the check must not demand (or perform) a wipe, and comparing by
//! uuid still catches an account created with no `external_ref`, which no
//! prefix sweep could. Both are `#[ignore]` so a plain `cargo test` never runs
//! one without the other.

use std::collections::BTreeSet;
use std::path::PathBuf;

use wamux::storage::StorageEngine;

// Only a subset of the shared helpers is used per test binary.
#[allow(dead_code)]
mod common;

const SNAPSHOT_ENV: &str = "WAMUX_ACCOUNT_SNAPSHOT";

fn snapshot_path() -> PathBuf {
    std::env::var_os(SNAPSHOT_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{SNAPSHOT_ENV} must name the snapshot file"))
}

/// Every account and every `wamux_upgrade_*` database, one per line, prefixed
/// by its kind so the two namespaces cannot collide.
async fn current_rows() -> BTreeSet<String> {
    let engine = common::pg_engine(2).await;
    let mut rows: BTreeSet<String> = engine
        .list_accounts()
        .await
        .expect("list accounts")
        .into_iter()
        .map(|row| format!("account {} {:?}", row.uuid, row.external_ref))
        .collect();
    let databases: Vec<String> = sqlx::query_scalar(
        "SELECT datname FROM pg_database WHERE datname LIKE 'wamux\\_upgrade\\_%'",
    )
    .fetch_all(engine.pool())
    .await
    .expect("list throwaway databases");
    rows.extend(databases.into_iter().map(|name| format!("database {name}")));
    rows
}

#[tokio::test]
#[ignore = "run by scripts/ci.sh before the Postgres-backed suites"]
async fn snapshot_accounts_before_the_run() {
    let rows = current_rows().await;
    let mut body = rows.into_iter().collect::<Vec<_>>().join("\n");
    body.push('\n');
    std::fs::write(snapshot_path(), body).expect("write the snapshot");
}

#[tokio::test]
#[ignore = "run by scripts/ci.sh after the Postgres-backed suites"]
async fn no_account_outlives_the_run() {
    let path = snapshot_path();
    let before: BTreeSet<String> = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read the snapshot {}: {e}", path.display()))
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    let leftovers: Vec<String> = current_rows().await.difference(&before).cloned().collect();
    assert!(
        leftovers.is_empty(),
        "{} row(s) created during the run were never cleaned up:\n{}",
        leftovers.len(),
        leftovers.join("\n")
    );
}
