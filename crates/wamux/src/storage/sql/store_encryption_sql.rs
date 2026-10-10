//! Opening a store against its `store_encryption` row (#164), sqlx family. The
//! decision itself is `storage::store_encryption::resolve`; this file reads the
//! inputs, writes the mark, and runs the conversion the decision asks for
//! (#165).

use wacore::store::error::{Result, StoreError};

use super::SqlPool;
use crate::storage::blob_cipher::BlobCipher;
use crate::storage::convert::Direction;
use crate::storage::statements::store_encryption::{
    HAS_ACCOUNTS, HAS_PROGRESS, MARK_CONVERTING, MARK_ENCRYPTED, SELECT_STATE,
};
use crate::storage::store_encryption::{
    Resolution, StoredEncryption, cipher_for_decrypt, plan_rotation, resolve,
};
use crate::storage::store_key::StoreKey;

type StateRow = (String, Option<Vec<u8>>, Option<Vec<u8>>);

/// The cipher this store's backends must use, after refusing a key that does not
/// fit the store, marking a new store encrypted, and finishing (or doing) the
/// conversion of a store that already has accounts.
pub(super) async fn open_cipher(pool: &SqlPool, key: Option<&StoreKey>) -> Result<BlobCipher> {
    let stored = read_stored(pool).await?;
    let has_accounts = scalar_one_sql!(bool, pool, HAS_ACCOUNTS)?;
    let has_progress = scalar_one_sql!(bool, pool, HAS_PROGRESS)?;
    let resolution = resolve(&stored, has_accounts, has_progress, key)?;
    write_mark(pool, &resolution).await?;
    if resolution.convert {
        super::convert::run(pool, &resolution.cipher, Direction::Seal).await?;
        super::convert::scrub_old_plaintext(pool).await?;
    }
    Ok(resolution.cipher)
}

/// Turn an encrypted store back into plaintext, resuming an interrupted run.
/// Does not go through `resolve`: that refuses exactly the half-decrypted state
/// this has to open.
pub(super) async fn decrypt_store(pool: &SqlPool, key: &StoreKey) -> Result<usize> {
    let stored = read_stored(pool).await?;
    let cipher = cipher_for_decrypt(&stored, key)?;
    super::convert::run(pool, &cipher, Direction::Open).await
}

/// Re-seal an encrypted store under `new` (#166). The plan, which refuses before
/// anything is written, is made from the state read here, then the rotation
/// runs in its one transaction.
pub(super) async fn rotate_store(pool: &SqlPool, old: &StoreKey, new: &StoreKey) -> Result<usize> {
    let stored = read_stored(pool).await?;
    let has_progress = scalar_one_sql!(bool, pool, HAS_PROGRESS)?;
    let rotation = plan_rotation(&stored, has_progress, old, new)?;
    super::rotate::run(pool, &rotation).await
}

async fn read_stored(pool: &SqlPool) -> Result<StoredEncryption> {
    let row = row_optional_sql!(StateRow, pool, SELECT_STATE)?
        .ok_or_else(|| StoreError::InvalidConfig("store_encryption has no row".into()))?;
    StoredEncryption::from_row(&row.0, row.1, row.2)
}

/// A new store is marked encrypted at once; a store to convert only records its
/// key and stays plaintext until its last account is sealed.
async fn write_mark(pool: &SqlPool, resolution: &Resolution) -> Result<()> {
    let Some(mark) = &resolution.mark else {
        return Ok(());
    };
    let statement = if resolution.convert {
        MARK_CONVERTING
    } else {
        MARK_ENCRYPTED
    };
    let marked = execute_sql!(pool, statement, &mark.key_id[..], &mark.verifier)?;
    if marked != 1 {
        return Err(StoreError::InvalidConfig(
            "the store changed while it was being marked for encryption".into(),
        ));
    }
    Ok(())
}
