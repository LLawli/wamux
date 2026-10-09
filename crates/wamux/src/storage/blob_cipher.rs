//! Seals and opens the blobs of the store's secret columns (#164).
//!
//! Layout of a sealed blob: `[version 1B = 1][key_id 4B][nonce 24B][ciphertext
//! and 16B tag]`, XChaCha20-Poly1305 with a fresh random nonce per seal. The
//! associated data binds the blob to where it lives (device, table, column and
//! row key), so a blob copied to another row or account fails to open instead
//! of being read as that row's secret.
//!
//! An EMPTY value stays empty in both directions: it carries no secret, and the
//! `tc_tokens` queries test `length(token) = 0` in SQL (an empty token is the
//! sentinel `touch_sender_timestamp` writes).

use chacha20poly1305::aead::{Aead, Generate, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use wacore::store::error::{Result as StoreResult, StoreError};

use super::store_key::{StoreKey, key_id_hex};

/// Layout version byte of a sealed blob.
const BLOB_VERSION: u8 = 1;
/// `[version 1B][key_id 4B][nonce 24B]`.
const HEADER_LEN: usize = 1 + 4 + 24;
const TAG_LEN: usize = 16;

/// Where a blob lives, i.e. what it is bound to.
#[derive(Clone, Copy, Debug)]
pub struct BlobContext<'a> {
    pub device_id: i32,
    pub table: &'static str,
    pub column: &'static str,
    /// The row's key bytes (see the scope of #164 for each table).
    pub row: &'a [u8],
}

/// A row key made of several parts (`chat`, `sender`, `msg_id`...), joined by a
/// zero byte so `("ab", "c")` and `("a", "bc")` are different rows to the AAD.
pub fn joined_row(parts: &[&[u8]]) -> Vec<u8> {
    parts.join(&0u8)
}

/// The cipher a backend holds: `passthrough` for a store with no key (the
/// bytes go in and out untouched), or `sealing` under a `StoreKey`.
#[derive(Clone)]
pub struct BlobCipher {
    key: Option<StoreKey>,
}

impl BlobCipher {
    pub fn passthrough() -> Self {
        Self { key: None }
    }

    pub fn sealing(key: &StoreKey) -> Self {
        Self {
            key: Some(key.clone()),
        }
    }

    pub fn is_sealing(&self) -> bool {
        self.key.is_some()
    }

    /// `seal` for a call site that has the pieces and no `BlobContext` yet.
    pub fn seal_at(
        &self,
        device_id: i32,
        table: &'static str,
        column: &'static str,
        row: &[u8],
        plaintext: &[u8],
    ) -> StoreResult<Vec<u8>> {
        self.seal(
            &BlobContext {
                device_id,
                table,
                column,
                row,
            },
            plaintext,
        )
    }

    /// `open` for a call site that has the pieces and no `BlobContext` yet.
    pub fn open_at(
        &self,
        device_id: i32,
        table: &'static str,
        column: &'static str,
        row: &[u8],
        stored: &[u8],
    ) -> StoreResult<Vec<u8>> {
        self.open(
            &BlobContext {
                device_id,
                table,
                column,
                row,
            },
            stored,
        )
    }

    /// Seal `plaintext` for `context`. Empty in, empty out. A passthrough
    /// cipher returns the bytes unchanged.
    pub fn seal(&self, context: &BlobContext<'_>, plaintext: &[u8]) -> StoreResult<Vec<u8>> {
        let Some(key) = &self.key else {
            return Ok(plaintext.to_vec());
        };
        if plaintext.is_empty() {
            return Ok(Vec::new());
        }
        let nonce = XNonce::try_generate()
            .map_err(|e| StoreError::Validation(format!("no randomness for a nonce: {e}")))?;
        let sealed = aead_for(key)?
            .encrypt(&nonce, payload(plaintext, &context_aad(context)))
            .map_err(|_| StoreError::Validation(format!("cannot seal {}", describe(context))))?;
        let mut blob = Vec::with_capacity(HEADER_LEN + sealed.len());
        blob.push(BLOB_VERSION);
        blob.extend_from_slice(&key.id());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&sealed);
        Ok(blob)
    }

    /// Open a blob read from `context`. Empty in, empty out. Fails, never
    /// panics, on a wrong key, a different context, an unknown version, a
    /// truncated blob or a bad tag.
    pub fn open(&self, context: &BlobContext<'_>, stored: &[u8]) -> StoreResult<Vec<u8>> {
        let Some(key) = &self.key else {
            return Ok(stored.to_vec());
        };
        if stored.is_empty() {
            return Ok(Vec::new());
        }
        let (nonce, sealed) = split_blob(key, context, stored)?;
        aead_for(key)?
            .decrypt(&nonce, payload(sealed, &context_aad(context)))
            .map_err(|_| {
                StoreError::Validation(format!(
                    "{} did not authenticate: moved to another row or account, or corrupted",
                    describe(context)
                ))
            })
    }
}

/// Check the header and cut the blob into its nonce and ciphertext+tag. Every
/// short or foreign blob ends here as an error, before any AEAD call.
fn split_blob<'b>(
    key: &StoreKey,
    context: &BlobContext<'_>,
    stored: &'b [u8],
) -> StoreResult<(XNonce, &'b [u8])> {
    if stored.len() < HEADER_LEN + TAG_LEN {
        return Err(StoreError::Validation(format!(
            "{} is {} bytes, shorter than a sealed blob ({} at least)",
            describe(context),
            stored.len(),
            HEADER_LEN + TAG_LEN
        )));
    }
    if stored[0] != BLOB_VERSION {
        return Err(StoreError::Validation(format!(
            "{} has sealed-blob version {}, this build reads version {BLOB_VERSION}",
            describe(context),
            stored[0]
        )));
    }
    if stored[1..5] != key.id() {
        return Err(StoreError::Validation(format!(
            "{} was sealed with key id {}, the configured key is {}",
            describe(context),
            key_id_hex(&[stored[1], stored[2], stored[3], stored[4]]),
            key_id_hex(&key.id())
        )));
    }
    let nonce = XNonce::try_from(&stored[5..HEADER_LEN])
        .map_err(|_| StoreError::Validation("sealed blob nonce has the wrong size".into()))?;
    Ok((nonce, &stored[HEADER_LEN..]))
}

fn aead_for(key: &StoreKey) -> StoreResult<XChaCha20Poly1305> {
    XChaCha20Poly1305::new_from_slice(key.expose())
        .map_err(|_| StoreError::InvalidConfig("store key has the wrong length".into()))
}

fn payload<'m, 'a>(msg: &'m [u8], aad: &'a [u8]) -> Payload<'m, 'a> {
    Payload { msg, aad }
}

/// device (4B big-endian) | table | 0 | column | 0 | row. The zero separators
/// keep `("ab", "c")` and `("a", "bc")` from producing the same bytes.
fn context_aad(context: &BlobContext<'_>) -> Vec<u8> {
    let mut aad =
        Vec::with_capacity(8 + context.table.len() + context.column.len() + context.row.len());
    aad.extend_from_slice(&context.device_id.to_be_bytes());
    aad.extend_from_slice(context.table.as_bytes());
    aad.push(0);
    aad.extend_from_slice(context.column.as_bytes());
    aad.push(0);
    aad.extend_from_slice(context.row);
    aad
}

/// Where a blob lives, for an error message. The row key is left out: it can
/// be a JID or a message id.
fn describe(context: &BlobContext<'_>) -> String {
    format!(
        "{}.{} of device {}",
        context.table, context.column, context.device_id
    )
}

#[cfg(test)]
#[path = "blob_cipher_tests.rs"]
mod tests;
