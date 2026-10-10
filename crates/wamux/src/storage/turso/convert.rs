//! Converting a store account by account (#165), turso family: the same walk as
//! `sql::convert` over the same `sealed_columns()` list and the same shared
//! statements, on the one connection. One transaction per account, holding the
//! connection from BEGIN to COMMIT.

use ::turso::Value;
use wacore::store::error::{Result as StoreResult, StoreError};

use super::TursoConn;
use super::row_values::{blob, int, int32, text};
use super::transaction::TursoTx;
use crate::storage::blob_cipher::BlobCipher;
use crate::storage::convert::{Direction, Turn, accounts_left, turn_blob};
use crate::storage::sealed_columns::{KeyColumn, RowPart, SealedColumn, sealed_columns};
use crate::storage::statements::convert::{
    CLEAR_PROGRESS, INSERT_PROGRESS, SELECT_ACCOUNT_IDS, SELECT_PROGRESS, VACUUM_FILE,
    WAL_TRUNCATE, select_rows, update_row,
};

/// One stored row of a column: the values of its key columns, then its blob.
type StoredRow = (Vec<RowPart>, Vec<u8>);

/// Turn every account that is not done yet; returns how many this run did.
pub(super) async fn run(
    conn: &TursoConn,
    cipher: &BlobCipher,
    direction: Direction,
) -> StoreResult<usize> {
    let all = device_ids(conn, SELECT_ACCOUNT_IDS).await?;
    let done = device_ids(conn, SELECT_PROGRESS).await?;
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
        convert_account(conn, cipher, direction, *device_id, is_last).await?;
    }
    if left.is_empty() {
        finish_alone(conn, direction).await?;
    }
    tracing::info!(accounts = left.len(), "store {} finished", direction.verb());
    Ok(left.len())
}

async fn device_ids(conn: &TursoConn, sql: &str) -> StoreResult<Vec<i32>> {
    let rows = conn.fetch_all(sql, Vec::new()).await?;
    rows.iter().map(|row| int32(row, 0)).collect()
}

async fn convert_account(
    conn: &TursoConn,
    cipher: &BlobCipher,
    direction: Direction,
    device_id: i32,
    is_last: bool,
) -> StoreResult<()> {
    let tx = TursoTx::begin(conn).await?;
    let turn = Turn::of(direction, cipher);
    for column in sealed_columns() {
        convert_column(&tx, &turn, device_id, column).await?;
    }
    tx.execute(INSERT_PROGRESS, vec![Value::Integer(i64::from(device_id))])
        .await?;
    if is_last {
        finish(&tx, direction).await?;
    }
    tx.commit().await
}

/// The state flip and the cleared progress, in the last account's transaction.
async fn finish(tx: &TursoTx<'_>, direction: Direction) -> StoreResult<()> {
    tx.execute(direction.finish_statement(), Vec::new()).await?;
    tx.execute(CLEAR_PROGRESS, Vec::new()).await?;
    Ok(())
}

/// Nothing was left to turn: the flip still has to happen.
async fn finish_alone(conn: &TursoConn, direction: Direction) -> StoreResult<()> {
    let tx = TursoTx::begin(conn).await?;
    finish(&tx, direction).await?;
    tx.commit().await
}

/// Turn every row of one column of one account inside `tx`. The rotation of
/// #166 calls this for all accounts in its single transaction.
pub(super) async fn convert_column(
    tx: &TursoTx<'_>,
    turn: &Turn<'_>,
    device_id: i32,
    column: &SealedColumn,
) -> StoreResult<()> {
    let device = Value::Integer(i64::from(device_id));
    let fetched = tx
        .fetch_all(&select_rows(column), vec![device.clone()])
        .await?;
    let update = update_row(column);
    for row in &fetched {
        let (parts, stored) = read_row(row, column)?;
        let turned = turn_blob(turn, device_id, column, &parts, &stored)?;
        let mut binds = vec![Value::Blob(turned), device.clone()];
        binds.extend(parts.iter().map(part_value));
        tx.execute(&update, binds).await?;
    }
    Ok(())
}

fn part_value(part: &RowPart) -> Value {
    match part {
        RowPart::Text(text) => Value::Text(text.clone()),
        RowPart::Int(id) => Value::Integer(*id),
        RowPart::Bytes(bytes) => Value::Blob(bytes.clone()),
    }
}

/// Decode one row: the key columns in list order, then the blob.
fn read_row(row: &::turso::Row, column: &SealedColumn) -> StoreResult<StoredRow> {
    let mut parts = Vec::with_capacity(column.key.len());
    for (index, key) in column.key.iter().enumerate() {
        parts.push(match key {
            KeyColumn::Text(_) => RowPart::Text(text(row, index)?),
            KeyColumn::Id(_) => RowPart::Int(int(row, index)?),
            KeyColumn::Bytes(_) => RowPart::Bytes(blob(row, index)?),
        });
    }
    Ok((parts, blob(row, column.key.len())?))
}

/// After a conversion to encrypted (#165): VACUUM drops the free pages that
/// still hold the old plaintext, and a truncating checkpoint drops the WAL
/// pages that do. Neither alone is enough.
pub(super) async fn scrub_old_plaintext(conn: &TursoConn) -> StoreResult<()> {
    conn.fetch_all(VACUUM_FILE, Vec::new()).await?;
    let checkpoint = conn.fetch_all(WAL_TRUNCATE, Vec::new()).await?;
    let busy = checkpoint
        .first()
        .map(|row| int(row, 0))
        .transpose()?
        .unwrap_or(0);
    if busy != 0 {
        return Err(StoreError::Connection(
            "the WAL could not be truncated after the secret columns were rewritten, so their \
             old bytes may still be in the -wal file: stop whatever else has the store open \
             and run again"
                .into(),
        ));
    }
    Ok(())
}
