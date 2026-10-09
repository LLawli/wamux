//! Opening a store against its `store_encryption` row (#164), sqlx family. The
//! decision itself is `storage::store_encryption::resolve`; this file only
//! reads the inputs and writes the mark.

use wacore::store::error::{Result, StoreError};

use super::SqlPool;
use crate::storage::blob_cipher::BlobCipher;
use crate::storage::statements::store_encryption::{HAS_ACCOUNTS, MARK_ENCRYPTED, SELECT_STATE};
use crate::storage::store_encryption::{StoredEncryption, resolve};
use crate::storage::store_key::StoreKey;

type StateRow = (String, Option<Vec<u8>>, Option<Vec<u8>>);

/// The cipher this store's backends must use, after refusing a key that does not
/// fit the store and marking a new store encrypted.
pub(super) async fn open_cipher(pool: &SqlPool, key: Option<&StoreKey>) -> Result<BlobCipher> {
    let row = row_optional_sql!(StateRow, pool, SELECT_STATE)?
        .ok_or_else(|| StoreError::InvalidConfig("store_encryption has no row".into()))?;
    let stored = StoredEncryption::from_row(&row.0, row.1, row.2)?;
    let has_accounts = scalar_one_sql!(bool, pool, HAS_ACCOUNTS)?;
    let resolution = resolve(&stored, has_accounts, key)?;
    if let Some(mark) = resolution.mark {
        let marked = execute_sql!(pool, MARK_ENCRYPTED, &mark.key_id[..], &mark.verifier)?;
        if marked != 1 {
            return Err(StoreError::InvalidConfig(
                "an account was created while the store was being marked encrypted".into(),
            ));
        }
    }
    Ok(resolution.cipher)
}
