//! `DeviceStore::maintenance` (#104). One of the few places with code per
//! driver, written side by side: the two engines keep themselves in shape in
//! different ways. Kept out of `device_store.rs` for the `*_store.rs` rule.

use sqlx::SqlitePool;
use wacore::store::error::Result;

use super::SqlPool;
use crate::storage::sqlx_error::db;

/// SQLite: refresh planner statistics and shrink the `-wal` file. Postgres has
/// nothing for the client to do: autovacuum owns statistics and dead tuples,
/// and a manual VACUUM or ANALYZE from here would only compete with it.
pub(super) async fn run(pool: &SqlPool) -> Result<()> {
    match pool {
        SqlPool::Pg(_) => Ok(()),
        SqlPool::Sqlite(pool) => sqlite_upkeep(pool).await,
    }
}

/// Mirrors the upstream `SqliteStore::maintenance`. The pragmas run on ONE
/// connection because `analysis_limit` is per connection and `optimize` must
/// see it.
async fn sqlite_upkeep(pool: &SqlitePool) -> Result<()> {
    let mut conn = pool.acquire().await.map_err(db)?;
    // Caps the index rows each ANALYZE samples, so the first `optimize` over a
    // large table stays in milliseconds and is safe on a live connection.
    sqlx::query("PRAGMA analysis_limit = 400")
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    // A no-op unless a table changed materially since the last ANALYZE.
    sqlx::query("PRAGMA optimize")
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    // TRUNCATE is the only checkpoint mode that returns the -wal file's blocks
    // to the filesystem. It declines rather than blocks while a reader holds a
    // snapshot, so a skipped truncate is the normal outcome, never a failure.
    let outcome = sqlx::query_as::<_, (i64, i64, i64)>("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_one(&mut *conn)
        .await;
    tolerate_busy(outcome)
}

/// Swallow only "busy": as `busy = 1` in the result row, or as SQLITE_BUSY
/// (primary code 5, extended codes keep it in the low byte). An I/O error, a
/// full disk or a permission problem means the log could not be written back,
/// which is what this pass exists to catch, so those stay errors.
fn tolerate_busy(outcome: std::result::Result<(i64, i64, i64), sqlx::Error>) -> Result<()> {
    match outcome {
        Ok(_busy_log_checkpointed) => Ok(()),
        Err(e) if is_sqlite_busy(&e) => Ok(()),
        Err(e) => Err(db(e)),
    }
}

fn is_sqlite_busy(error: &sqlx::Error) -> bool {
    let code = error.as_database_error().and_then(|d| d.code());
    let primary = code.and_then(|c| c.parse::<i32>().ok()).map(|c| c & 0xff);
    primary == Some(5)
}
