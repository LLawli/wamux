//! The one `SignalStore` statement with no common form across the drivers.
//! Kept out of `signal_store.rs`: the coverage checks read every `async fn` in
//! `*_store.rs` as a trait method.

use sqlx::PgPool;
use wacore::store::error::Result;

use super::{SqlPool, SqliteConn, end_job};
use crate::storage::sqlx_error::db;
use crate::storage::statements::signal::MARK_PREKEY_UPLOADED;

/// Mark the prekeys as uploaded. Written per driver, side by side, because
/// Postgres binds one array (`= ANY($2)`) while SQLite has no array bind and
/// takes one statement per id, in one transaction. The SQLite statement is in
/// `statements::signal`, shared with Turso (#106); the array one stays here.
pub(super) async fn mark_uploaded(pool: &SqlPool, device_id: i32, ids: &[u32]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    match pool {
        SqlPool::Pg(pool) => mark_uploaded_array(pool, device_id, ids).await,
        SqlPool::Sqlite(lite) => mark_uploaded_each(lite, device_id, ids).await,
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

/// One job with the loop inside: the ids commit together or not at all.
async fn mark_uploaded_each(lite: &SqliteConn, device_id: i32, ids: &[u32]) -> Result<()> {
    let mut job = lite.begin_job().await?;
    let mut ran: Result<()> = Ok(());
    for id in ids {
        let marked = sqlx::query(MARK_PREKEY_UPLOADED)
            .bind(*id as i32)
            .bind(device_id)
            .execute(&mut *job)
            .await;
        if let Err(error) = marked {
            ran = Err(db(error));
            break;
        }
    }
    end_job(job, ran).await
}
