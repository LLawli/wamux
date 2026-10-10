//! Re-sealing a whole store under a new key (#166), turso family: the same walk
//! as `sql::rotate` over `convert::convert_column`, in ONE transaction held on
//! the one connection from BEGIN to COMMIT.

use ::turso::Value;
use wacore::store::error::{Result as StoreResult, StoreError};

use super::TursoConn;
use super::convert::{convert_column, scrub_old_plaintext};
use super::row_values::int32;
use super::transaction::TursoTx;
use crate::storage::convert::Turn;
use crate::storage::sealed_columns::sealed_columns;
use crate::storage::statements::convert::SELECT_ACCOUNT_IDS;
use crate::storage::statements::store_encryption::ROTATE_MARK;
use crate::storage::store_encryption::Rotation;

/// Rotate every account and the mark in one transaction, then scrub the old
/// blobs out of the file. Returns how many accounts were rotated.
pub(super) async fn run(conn: &TursoConn, rotation: &Rotation) -> StoreResult<usize> {
    let rows = conn.fetch_all(SELECT_ACCOUNT_IDS, Vec::new()).await?;
    let device_ids: Vec<i32> = rows
        .iter()
        .map(|row| int32(row, 0))
        .collect::<StoreResult<_>>()?;
    let turn = Turn::Rotate {
        from: &rotation.from,
        to: &rotation.to,
    };
    let tx = TursoTx::begin(conn).await?;
    for device_id in &device_ids {
        tracing::info!(device_id, "rotating account");
        for column in sealed_columns() {
            convert_column(&tx, &turn, *device_id, column).await?;
        }
    }
    swap_mark(&tx, rotation).await?;
    tx.commit().await?;
    scrub_old_plaintext(conn).await?;
    tracing::info!(accounts = device_ids.len(), "store rotation finished");
    Ok(device_ids.len())
}

async fn swap_mark(tx: &TursoTx<'_>, rotation: &Rotation) -> StoreResult<()> {
    let mark = &rotation.mark;
    let binds = vec![
        Value::Blob(mark.key_id.to_vec()),
        Value::Blob(mark.verifier.clone()),
    ];
    if tx.execute(ROTATE_MARK, binds).await? != 1 {
        return Err(StoreError::InvalidConfig(
            "the store stopped being encrypted while its key was being rotated".into(),
        ));
    }
    Ok(())
}
