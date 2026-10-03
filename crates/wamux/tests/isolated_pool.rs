//! #107: Postgres connections the pool opens while a `run_isolated` call runs
//! must stay usable once it returns. They used to be bound to the call's
//! throwaway runtime, so every later acquire that picked one waited out
//! `acquire_timeout` (30 s) and failed: signal flush, sessions and sends on the
//! main runtime all stalled. SQLite is not affected (sqlx-sqlite runs each
//! connection on its own thread, not on tokio's I/O driver), hence Postgres only.

// Only a subset of the shared helpers is used here.
#[allow(dead_code)]
mod common;

use std::time::Duration;

use sqlx::PgPool;
use wamux::domain::isolate::run_isolated;
use wamux::storage::sql::connect_postgres;

/// Well under `acquire_timeout` (30 s): an orphaned connection fails this,
/// a healthy pool answers in milliseconds.
const PATIENCE: Duration = Duration::from_secs(10);

/// Concurrent sleeps inside one isolated call, so the pool has to open
/// connections there (one query per connection at a time).
async fn queries_inside_run_isolated(pool: &PgPool, n: usize) {
    let pool = pool.clone();
    run_isolated(move || async move {
        let sleeps = (0..n).map(|_| {
            let pool = pool.clone();
            async move { sqlx::query("SELECT pg_sleep(0.05)").execute(&pool).await }
        });
        let results = futures::future::join_all(sleeps).await;
        results.into_iter().collect::<Result<Vec<_>, _>>()
    })
    .await
    .expect("queries inside run_isolated");
}

/// Concurrent queries on the caller's runtime; every one must succeed in time.
async fn queries_on_the_main_runtime(pool: &PgPool, n: usize) {
    let selects = (0..n).map(|_| {
        let pool = pool.clone();
        async move { sqlx::query("SELECT 1").execute(&pool).await }
    });
    let results = tokio::time::timeout(PATIENCE, futures::future::join_all(selects))
        .await
        .expect("main-runtime queries must not wait out the pool's acquire_timeout");
    for result in results {
        result.expect("main-runtime query after run_isolated");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn postgres_pool_stays_usable_after_run_isolated() {
    let pool = connect_postgres(&common::database_url(), 4)
        .await
        .expect("connect postgres");
    for round in 0..3 {
        queries_inside_run_isolated(&pool, 4).await;
        queries_on_the_main_runtime(&pool, 8).await;
        assert!(pool.size() <= 4, "round {round}: pool size {}", pool.size());
    }
    pool.close().await;
}
