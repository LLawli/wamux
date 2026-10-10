//! Re-sealing a whole store under a new key (#166), sqlx family. The walk over
//! the 13 columns is `convert::convert_column`; what differs from a conversion
//! is the transform (open with the old key, seal with the new) and the
//! transaction: ONE for every account and the mark swap, so an interruption
//! leaves the store whole under the old key and there is no half-rotated state
//! to resume from.

use wacore::store::error::{Result as StoreResult, StoreError};

use super::convert::{convert_column, scrub_old_plaintext};
use super::{SqlPool, SqlTx};
use crate::storage::convert::Turn;
use crate::storage::sealed_columns::sealed_columns;
use crate::storage::statements::convert::SELECT_ACCOUNT_IDS;
use crate::storage::statements::store_encryption::ROTATE_MARK;
use crate::storage::store_encryption::Rotation;

/// Rotate every account and the mark in one transaction, then scrub the old
/// blobs out of the storage. Returns how many accounts were rotated.
pub(super) async fn run(pool: &SqlPool, rotation: &Rotation) -> StoreResult<usize> {
    let mut device_ids = scalar_all_sql!(i32, pool, SELECT_ACCOUNT_IDS)?;
    device_ids.sort_unstable();
    let turn = Turn::Rotate {
        from: &rotation.from,
        to: &rotation.to,
    };
    let mut tx = SqlTx::begin(pool).await?;
    for device_id in &device_ids {
        tracing::info!(device_id, "rotating account");
        for column in sealed_columns() {
            convert_column(&mut tx, &turn, *device_id, column).await?;
        }
    }
    swap_mark(&mut tx, rotation).await?;
    tx.commit().await?;
    // After the commit and outside any transaction: VACUUM FULL cannot run in one.
    scrub_old_plaintext(pool).await?;
    tracing::info!(accounts = device_ids.len(), "store rotation finished");
    Ok(device_ids.len())
}

async fn swap_mark(mut tx: &mut SqlTx<'static>, rotation: &Rotation) -> StoreResult<()> {
    let mark = &rotation.mark;
    let swapped = execute_sql!(in tx, ROTATE_MARK, &mark.key_id[..], &mark.verifier)?;
    if swapped != 1 {
        return Err(StoreError::InvalidConfig(
            "the store stopped being encrypted while its key was being rotated".into(),
        ));
    }
    Ok(())
}
