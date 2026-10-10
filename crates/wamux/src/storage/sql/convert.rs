//! Converting a store account by account (#165), sqlx family: sealing the
//! secret columns when a key is turned on over existing accounts, and opening
//! them again for `wamux store decrypt`.
//!
//! One transaction per account: its 13 columns, its progress row and, for the
//! last account, the state flip. An interruption therefore leaves whole
//! accounts done and whole accounts untouched, and the retry skips the done
//! ones by their progress row. The columns and the row keys come from
//! `sealed_columns()`; the SQL from `statements::convert`.

use sqlx::{ColumnIndex, Decode, Row, Type};
use wacore::store::error::{Result as StoreResult, StoreError};

use super::{SqlPool, SqlTx};
use crate::storage::blob_cipher::BlobCipher;
use crate::storage::convert::{Direction, accounts_left, turn_blob};
use crate::storage::sealed_columns::{KeyColumn, RowPart, SealedColumn, sealed_columns};
use crate::storage::sqlx_error::db;
use crate::storage::statements::convert::{
    CLEAR_PROGRESS, INSERT_PROGRESS, SELECT_ACCOUNT_IDS, SELECT_PROGRESS, VACUUM_FILE,
    WAL_TRUNCATE, select_rows, update_row, vacuum_full,
};

/// One stored row of a column: the values of its key columns, then its blob.
type StoredRow = (Vec<RowPart>, Vec<u8>);

/// Turn every account that is not done yet; returns how many this run did.
pub(super) async fn run(
    pool: &SqlPool,
    cipher: &BlobCipher,
    direction: Direction,
) -> StoreResult<usize> {
    let all = scalar_all_sql!(i32, pool, SELECT_ACCOUNT_IDS)?;
    let done = scalar_all_sql!(i32, pool, SELECT_PROGRESS)?;
    let left = accounts_left(all, &done);
    for (index, device_id) in left.iter().enumerate() {
        let is_last = index + 1 == left.len();
        tracing::info!(
            device_id,
            position = index + 1,
            total = left.len(),
            "{} account",
            direction.verb()
        );
        convert_account(pool, cipher, direction, *device_id, is_last).await?;
    }
    if left.is_empty() {
        finish_alone(pool, direction).await?;
    }
    tracing::info!(accounts = left.len(), "store {} finished", direction.verb());
    Ok(left.len())
}

async fn convert_account(
    pool: &SqlPool,
    cipher: &BlobCipher,
    direction: Direction,
    device_id: i32,
    is_last: bool,
) -> StoreResult<()> {
    let mut tx = SqlTx::begin(pool).await?;
    for column in sealed_columns() {
        convert_column(&mut tx, cipher, direction, device_id, column).await?;
    }
    execute_sql!(in tx, INSERT_PROGRESS, device_id)?;
    if is_last {
        finish(&mut tx, direction).await?;
    }
    tx.commit().await
}

/// The state flip and the cleared progress, in the last account's transaction.
async fn finish(mut tx: &mut SqlTx<'static>, direction: Direction) -> StoreResult<()> {
    execute_sql!(in tx, direction.finish_statement())?;
    execute_sql!(in tx, CLEAR_PROGRESS)?;
    Ok(())
}

/// Nothing was left to turn (every account was already done, or there are none):
/// the flip still has to happen.
async fn finish_alone(pool: &SqlPool, direction: Direction) -> StoreResult<()> {
    let mut tx = SqlTx::begin(pool).await?;
    finish(&mut tx, direction).await?;
    tx.commit().await
}

async fn convert_column(
    mut tx: &mut SqlTx<'static>,
    cipher: &BlobCipher,
    direction: Direction,
    device_id: i32,
    column: &SealedColumn,
) -> StoreResult<()> {
    let select = select_rows(column);
    let update = update_row(column);
    let rows: Vec<StoredRow> = on_sql_tx!(tx, |conn| {
        let fetched = sqlx::query(&select).bind(device_id).fetch_all(conn).await;
        fetched.and_then(|rows| rows.iter().map(|row| read_row(row, column)).collect())
    })
    .map_err(db)?;
    for (parts, blob) in rows {
        let turned = turn_blob(cipher, direction, device_id, column, &parts, &blob)?;
        write_row(tx, &update, device_id, &parts, turned).await?;
    }
    Ok(())
}

async fn write_row(
    mut tx: &mut SqlTx<'static>,
    update: &str,
    device_id: i32,
    parts: &[RowPart],
    turned: Vec<u8>,
) -> StoreResult<()> {
    on_sql_tx!(tx, |conn| {
        let mut query = sqlx::query(update).bind(turned).bind(device_id);
        for part in parts {
            query = match part {
                RowPart::Text(text) => query.bind(text.clone()),
                RowPart::Int(id) => query.bind(*id),
                RowPart::Bytes(bytes) => query.bind(bytes.clone()),
            };
        }
        query.execute(conn).await.map(|done| done.rows_affected())
    })
    .map(drop)
    .map_err(db)
}

/// Decode one row: the key columns in list order, then the blob.
fn read_row<R>(row: &R, column: &SealedColumn) -> sqlx::Result<StoredRow>
where
    R: Row,
    usize: ColumnIndex<R>,
    for<'r> String: Decode<'r, R::Database> + Type<R::Database>,
    for<'r> i32: Decode<'r, R::Database> + Type<R::Database>,
    for<'r> Vec<u8>: Decode<'r, R::Database> + Type<R::Database>,
{
    let mut parts = Vec::with_capacity(column.key.len());
    for (index, key) in column.key.iter().enumerate() {
        parts.push(match key {
            KeyColumn::Text(_) => RowPart::Text(row.try_get(index)?),
            KeyColumn::Id(_) => RowPart::Int(i64::from(row.try_get::<i32, _>(index)?)),
            KeyColumn::Bytes(_) => RowPart::Bytes(row.try_get(index)?),
        });
    }
    let blob: Vec<u8> = row.try_get(column.key.len())?;
    Ok((parts, blob))
}

/// After a conversion to encrypted: get the old plaintext out of the store's
/// storage, where an UPDATE leaves it (#165). SQLite: the freed pages and the
/// WAL keep it, so VACUUM and then a truncating checkpoint. Postgres: the dead
/// tuples keep it, so VACUUM FULL rewrites each table (outside a transaction;
/// it takes a lock, which is why the daemon is not serving yet). A backup or a
/// replica made before is out of reach, and the docs say so.
pub(super) async fn scrub_old_plaintext(pool: &SqlPool) -> StoreResult<()> {
    match pool {
        SqlPool::Pg(pool) => {
            for column in sealed_columns() {
                sqlx::query(&vacuum_full(column))
                    .execute(pool)
                    .await
                    .map_err(db)?;
            }
            Ok(())
        }
        SqlPool::Sqlite(pool) => scrub_sqlite(pool).await,
    }
}

async fn scrub_sqlite(pool: &sqlx::SqlitePool) -> StoreResult<()> {
    sqlx::query(VACUUM_FILE).execute(pool).await.map_err(db)?;
    let (busy, _log_frames, _moved) = sqlx::query_as::<_, (i64, i64, i64)>(WAL_TRUNCATE)
        .fetch_one(pool)
        .await
        .map_err(db)?;
    if busy != 0 {
        return Err(StoreError::Connection(
            "the WAL could not be truncated after the conversion, so the old plaintext may \
             still be in the -wal file: stop whatever else has the store open and run again"
                .into(),
        ));
    }
    Ok(())
}
