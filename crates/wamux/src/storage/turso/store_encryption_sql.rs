//! Opening a store against its `store_encryption` row (#164), turso family. The
//! decision itself is `storage::store_encryption::resolve`; this file reads the
//! inputs, writes the mark, and runs the conversion the decision asks for
//! (#165).

use wacore::store::error::{Result, StoreError};

use super::TursoConn;
use super::exec::binds;
use super::row_values::{int, opt_blob, text};
use crate::storage::blob_cipher::BlobCipher;
use crate::storage::convert::Direction;
use crate::storage::statements::store_encryption::{
    HAS_ACCOUNTS, HAS_PROGRESS, MARK_CONVERTING, MARK_ENCRYPTED, SELECT_STATE,
};
use crate::storage::store_encryption::{Resolution, StoredEncryption, cipher_for_decrypt, resolve};
use crate::storage::store_key::StoreKey;

/// The cipher this store's backends must use, after refusing a key that does not
/// fit the store, marking a new store encrypted, and finishing (or doing) the
/// conversion of a store that already has accounts.
pub(super) async fn open_cipher(conn: &TursoConn, key: Option<&StoreKey>) -> Result<BlobCipher> {
    let stored = read_stored(conn).await?;
    let has_accounts = exists(conn, HAS_ACCOUNTS).await?;
    let has_progress = exists(conn, HAS_PROGRESS).await?;
    let resolution = resolve(&stored, has_accounts, has_progress, key)?;
    write_mark(conn, &resolution).await?;
    if resolution.convert {
        super::convert::run(conn, &resolution.cipher, Direction::Seal).await?;
        super::convert::scrub_old_plaintext(conn).await?;
    }
    Ok(resolution.cipher)
}

/// Turn an encrypted store back into plaintext, resuming an interrupted run.
/// Does not go through `resolve`: that refuses exactly the half-decrypted state
/// this has to open.
pub(super) async fn decrypt_store(conn: &TursoConn, key: &StoreKey) -> Result<usize> {
    let stored = read_stored(conn).await?;
    let cipher = cipher_for_decrypt(&stored, key)?;
    super::convert::run(conn, &cipher, Direction::Open).await
}

async fn read_stored(conn: &TursoConn) -> Result<StoredEncryption> {
    let row = conn
        .fetch_optional(SELECT_STATE, Vec::new())
        .await?
        .ok_or_else(|| StoreError::InvalidConfig("store_encryption has no row".into()))?;
    StoredEncryption::from_row(&text(&row, 0)?, opt_blob(&row, 1)?, opt_blob(&row, 2)?)
}

async fn exists(conn: &TursoConn, sql: &str) -> Result<bool> {
    let row = conn.fetch_optional(sql, Vec::new()).await?;
    Ok(row.map(|row| int(&row, 0)).transpose()? == Some(1))
}

/// A new store is marked encrypted at once; a store to convert only records its
/// key and stays plaintext until its last account is sealed.
async fn write_mark(conn: &TursoConn, resolution: &Resolution) -> Result<()> {
    let Some(mark) = &resolution.mark else {
        return Ok(());
    };
    let statement = if resolution.convert {
        MARK_CONVERTING
    } else {
        MARK_ENCRYPTED
    };
    let marked = conn
        .execute(statement, binds![&mark.key_id[..], mark.verifier.clone()])
        .await?;
    if marked != 1 {
        return Err(StoreError::InvalidConfig(
            "the store changed while it was being marked for encryption".into(),
        ));
    }
    Ok(())
}
