//! Opening a store against its `store_encryption` row (#164), turso family. The
//! decision itself is `storage::store_encryption::resolve`; this file only
//! reads the inputs and writes the mark.

use wacore::store::error::{Result, StoreError};

use super::TursoConn;
use super::exec::binds;
use super::row_values::{int, opt_blob, text};
use crate::storage::blob_cipher::BlobCipher;
use crate::storage::statements::store_encryption::{HAS_ACCOUNTS, MARK_ENCRYPTED, SELECT_STATE};
use crate::storage::store_encryption::{StoredEncryption, resolve};
use crate::storage::store_key::StoreKey;

/// The cipher this store's backends must use, after refusing a key that does not
/// fit the store and marking a new store encrypted.
pub(super) async fn open_cipher(conn: &TursoConn, key: Option<&StoreKey>) -> Result<BlobCipher> {
    let row = conn
        .fetch_optional(SELECT_STATE, Vec::new())
        .await?
        .ok_or_else(|| StoreError::InvalidConfig("store_encryption has no row".into()))?;
    let stored =
        StoredEncryption::from_row(&text(&row, 0)?, opt_blob(&row, 1)?, opt_blob(&row, 2)?)?;
    let accounts = conn.fetch_optional(HAS_ACCOUNTS, Vec::new()).await?;
    let has_accounts = accounts.map(|row| int(&row, 0)).transpose()? == Some(1);
    let resolution = resolve(&stored, has_accounts, key)?;
    if let Some(mark) = resolution.mark {
        let marked = conn
            .execute(MARK_ENCRYPTED, binds![&mark.key_id[..], mark.verifier])
            .await?;
        if marked != 1 {
            return Err(StoreError::InvalidConfig(
                "an account was created while the store was being marked encrypted".into(),
            ));
        }
    }
    Ok(resolution.cipher)
}
