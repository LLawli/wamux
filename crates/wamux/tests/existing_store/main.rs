//! #65: a store the pre-unification code wrote opens on the unified SQL and
//! keeps working. The fixtures are real stores written by `0f40e34` (see
//! tests/fixtures/store-0f40e34/README.md); the parity suite only proves the
//! new code reads back what the new code wrote.
//!
//! Each engine's half is a pair: `reads_back` checks every getter against
//! `values`, `keeps_working` writes, creates and deletes on top of the old rows.

// Only a subset of the shared helpers is used here.
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;

mod checks;
mod fixture;
mod values;

use sqlx::SqlitePool;

#[tokio::test]
async fn sqlite_store_written_before_unification_reads_back() {
    let (store, _dir) = fixture::sqlite().await;
    checks::reads_back(&store, fixture::SQLITE_ACCOUNT).await;
}

#[tokio::test]
async fn postgres_store_written_before_unification_reads_back() {
    let (store, db) = fixture::postgres().await;
    checks::reads_back(&store, fixture::POSTGRES_ACCOUNT).await;
    fixture::drop_postgres(store, db).await;
}

#[tokio::test]
async fn sqlite_store_written_before_unification_keeps_working() {
    let (store, _dir) = fixture::sqlite().await;
    checks::keeps_working(&store, fixture::SQLITE_ACCOUNT).await;
}

#[tokio::test]
async fn postgres_store_written_before_unification_keeps_working() {
    let (store, db) = fixture::postgres().await;
    checks::keeps_working(&store, fixture::POSTGRES_ACCOUNT).await;
    fixture::drop_postgres(store, db).await;
}

/// The premise of #65's "one `$N` string for both engines": sqlx-sqlite binds a
/// `$N` placeholder to argument N, so repeated and out-of-order placeholders
/// work (`sqlx-sqlite-0.8.6/src/arguments.rs`, the `strip_prefix('$')` arm). A
/// sqlx bump that changes this fails here, not in a store query.
#[tokio::test]
async fn sqlite_binds_dollar_placeholders_by_number() {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("open in-memory sqlite");
    let row: (i64, i64, i64) = sqlx::query_as("SELECT $2, $1, $2")
        .bind(10_i64)
        .bind(20_i64)
        .fetch_one(&pool)
        .await
        .expect("query with $N placeholders");
    assert_eq!(row, (20, 10, 20));
}
