//! The one `SignalStore` statement with no common form across the drivers.
//! Kept out of `signal_store.rs`: the coverage checks read every `async fn` in
//! `*_store.rs` as a trait method.

use sqlx::{PgPool, SqlitePool};
use wacore::store::error::Result;

use super::SqlPool;
use crate::storage::sqlx_error::db;

/// Mark the prekeys as uploaded. Written per driver, side by side, because
/// Postgres binds one array (`= ANY($2)`) while SQLite has no array bind and
/// takes one statement per id, in one transaction.
pub(super) async fn mark_uploaded(pool: &SqlPool, device_id: i32, ids: &[u32]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    match pool {
        SqlPool::Pg(pool) => mark_uploaded_array(pool, device_id, ids).await,
        SqlPool::Sqlite(pool) => mark_uploaded_each(pool, device_id, ids).await,
    }
}

async fn mark_uploaded_array(pool: &PgPool, device_id: i32, ids: &[u32]) -> Result<()> {
    let ids: Vec<i32> = ids.iter().map(|id| *id as i32).collect();
    sqlx::query("UPDATE prekeys SET uploaded = TRUE WHERE device_id = $1 AND id = ANY($2)")
        .bind(device_id)
        .bind(&ids)
        .execute(pool)
        .await
        .map_err(db)?;
    Ok(())
}

async fn mark_uploaded_each(pool: &SqlitePool, device_id: i32, ids: &[u32]) -> Result<()> {
    let mut tx = pool.begin().await.map_err(db)?;
    for id in ids {
        sqlx::query("UPDATE prekeys SET uploaded = TRUE WHERE id = $1 AND device_id = $2")
            .bind(*id as i32)
            .bind(device_id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
    }
    tx.commit().await.map_err(db)?;
    Ok(())
}
