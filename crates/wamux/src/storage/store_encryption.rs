//! Deciding, when a store opens, whether its secret columns are sealed and
//! with what (#164). Pure: the engines read the `store_encryption` row and the
//! account count, call `resolve`, and write the mark it asks for.
//!
//! The decision table (#165 added the conversion rows; the progress column is
//! `store_conversion_progress`, see migration 0006):
//!
//! | state     | verifier | progress | accounts | key   | result                         |
//! |-----------|----------|----------|----------|-------|--------------------------------|
//! | plaintext | none     | none     | any      | no    | plaintext, passthrough         |
//! | plaintext | none     | none     | none     | yes   | NEW store: mark encrypted      |
//! | plaintext | none     | none     | some     | yes   | CONVERT: mark the key, seal    |
//! | plaintext | set      | any      | any      | right | RESUME the conversion          |
//! | plaintext | set      | any      | any      | wrong | refused (does not match)       |
//! | plaintext | set      | any      | any      | no    | refused (interrupted)          |
//! | encrypted | set      | none     | any      | no    | refused                        |
//! | encrypted | set      | none     | any      | wrong | refused (does not match)       |
//! | encrypted | set      | none     | any      | right | sealing                        |
//! | encrypted | set      | some     | any      | any   | refused (decrypt interrupted)  |
//!
//! A verifier next to the state `plaintext` means a conversion has begun: the
//! key is written before the first account is sealed, so a different key on
//! the retry is caught here instead of sealing the rest under a second key.
//!
//! A wrong key is caught here, on the verifier, and not on the first read of
//! some account's session minutes later.

use wacore::store::error::{Result as StoreResult, StoreError};

use super::blob_cipher::{BlobCipher, BlobContext};
use super::store_key::{StoreKey, key_id_hex};

/// What the verifier seals: any fixed value works, a wrong key cannot open it.
const VERIFIER_PLAINTEXT: &[u8] = b"wamux store encryption verifier v1";

const VERIFIER_CONTEXT: BlobContext<'static> = BlobContext {
    device_id: 0,
    table: "store_encryption",
    column: "verifier",
    row: b"",
};

/// The `store_encryption` row as stored.
#[derive(Debug)]
pub(crate) struct StoredEncryption {
    pub encrypted: bool,
    pub key_id: Option<Vec<u8>>,
    pub verifier: Option<Vec<u8>>,
}

/// What to write when a new store is born encrypted.
#[derive(Debug)]
pub(crate) struct NewMark {
    pub key_id: [u8; 4],
    pub verifier: Vec<u8>,
}

/// The cipher the backends use, and the mark to write first if the store is new.
pub(crate) struct Resolution {
    pub cipher: BlobCipher,
    pub mark: Option<NewMark>,
    /// Seal every account still plaintext before the store is used (#165).
    /// With a `mark` the conversion is new, without one it resumes.
    pub convert: bool,
}

impl StoredEncryption {
    /// Read the `state` text of the row. Anything else cannot happen under the
    /// table's CHECK, but a store edited by hand is refused, not guessed at.
    pub(crate) fn from_row(
        state: &str,
        key_id: Option<Vec<u8>>,
        verifier: Option<Vec<u8>>,
    ) -> StoreResult<Self> {
        let encrypted = match state {
            "plaintext" => false,
            "encrypted" => true,
            other => {
                return Err(StoreError::InvalidConfig(format!(
                    "store_encryption.state is '{other}', expected 'plaintext' or 'encrypted'"
                )));
            }
        };
        Ok(Self {
            encrypted,
            key_id,
            verifier,
        })
    }
}

pub(crate) fn resolve(
    stored: &StoredEncryption,
    has_accounts: bool,
    has_progress: bool,
    key: Option<&StoreKey>,
) -> StoreResult<Resolution> {
    match (stored.encrypted, key) {
        (true, _) if has_progress => Err(decrypt_interrupted()),
        (true, None) => Err(StoreError::InvalidConfig(
            "this store is encrypted: set store_key_file (WAMUX_STORE_KEY_FILE) to the key file \
             it was created with"
                .into(),
        )),
        (true, Some(key)) => verify_key(stored, key),
        (false, key) if stored.verifier.is_some() => resume_conversion(stored, key),
        (false, None) => Ok(passthrough()),
        (false, Some(key)) if has_accounts => begin_conversion(key),
        (false, Some(key)) => new_encrypted_store(key),
    }
}

/// The cipher `wamux store decrypt` opens the blobs with (#165): the store must
/// be encrypted and the key must open its verifier. A half-finished decrypt
/// passes (state `encrypted`, progress rows), which is how it resumes.
pub(crate) fn cipher_for_decrypt(
    stored: &StoredEncryption,
    key: &StoreKey,
) -> StoreResult<BlobCipher> {
    if !stored.encrypted {
        return Err(StoreError::InvalidConfig(
            "this store is not encrypted, there is nothing to decrypt (if a conversion to \
             encrypted was interrupted, finish it by starting the daemon with its store_key_file)"
                .into(),
        ));
    }
    Ok(verify_key(stored, key)?.cipher)
}

/// What `wamux store rotate-key` turns every blob with (#166): `from` opens
/// what the old key sealed, `to` seals under the new one, and `mark` replaces
/// the store's `key_id` and verifier in the same transaction as the last blob.
pub(crate) struct Rotation {
    pub from: BlobCipher,
    pub to: BlobCipher,
    pub mark: NewMark,
}

/// Check that `old` is the key this store is encrypted with and that `new` is a
/// different one. A plaintext store (a half-done conversion included) and a
/// half-done decrypt are refused: the rotation only turns a whole store.
pub(crate) fn plan_rotation(
    stored: &StoredEncryption,
    has_progress: bool,
    old: &StoreKey,
    new: &StoreKey,
) -> StoreResult<Rotation> {
    if !stored.encrypted {
        return Err(StoreError::InvalidConfig(
            "this store is not encrypted, there is nothing to rotate (if a conversion to \
             encrypted was interrupted, finish it by starting the daemon with its store_key_file)"
                .into(),
        ));
    }
    if has_progress {
        return Err(decrypt_interrupted());
    }
    let from = verify_key(stored, old)?.cipher;
    if old.id() == new.id() {
        return Err(StoreError::InvalidConfig(format!(
            "the new store key (id {}) is already the key this store is encrypted with",
            key_id_hex(&new.id())
        )));
    }
    let mark = new_encrypted_store(new)?
        .mark
        .ok_or_else(|| StoreError::InvalidConfig("the new key produced no mark".into()))?;
    Ok(Rotation {
        from,
        to: BlobCipher::sealing(new),
        mark,
    })
}

fn decrypt_interrupted() -> StoreError {
    StoreError::InvalidConfig(
        "this store was being decrypted and the run was interrupted, so it is half plaintext: \
         finish it with `wamux store decrypt --yes` (with the same store_key_file) before \
         starting the daemon"
            .into(),
    )
}

fn resume_conversion(stored: &StoredEncryption, key: Option<&StoreKey>) -> StoreResult<Resolution> {
    let Some(key) = key else {
        return Err(StoreError::InvalidConfig(
            "this store was being converted to encrypted and the run was interrupted, so it is \
             half sealed: set store_key_file (WAMUX_STORE_KEY_FILE) to the key the conversion \
             started with to finish it"
                .into(),
        ));
    };
    let mut resolution = verify_key(stored, key)?;
    resolution.convert = true;
    Ok(resolution)
}

fn begin_conversion(key: &StoreKey) -> StoreResult<Resolution> {
    let mut resolution = new_encrypted_store(key)?;
    resolution.convert = true;
    Ok(resolution)
}

fn passthrough() -> Resolution {
    Resolution {
        cipher: BlobCipher::passthrough(),
        mark: None,
        convert: false,
    }
}

fn new_encrypted_store(key: &StoreKey) -> StoreResult<Resolution> {
    let cipher = BlobCipher::sealing(key);
    let verifier = cipher.seal(&VERIFIER_CONTEXT, VERIFIER_PLAINTEXT)?;
    Ok(Resolution {
        cipher,
        mark: Some(NewMark {
            key_id: key.id(),
            verifier,
        }),
        convert: false,
    })
}

fn verify_key(stored: &StoredEncryption, key: &StoreKey) -> StoreResult<Resolution> {
    let cipher = BlobCipher::sealing(key);
    let opened = stored
        .verifier
        .as_deref()
        .map(|verifier| cipher.open(&VERIFIER_CONTEXT, verifier));
    if matches!(&opened, Some(Ok(plain)) if plain == VERIFIER_PLAINTEXT) {
        return Ok(Resolution {
            cipher,
            mark: None,
            convert: false,
        });
    }
    let expected = stored
        .key_id
        .as_deref()
        .map_or_else(|| "unknown".to_string(), hex_of);
    Err(StoreError::InvalidConfig(format!(
        "the store key (id {}) does not match the key this store was encrypted with (id {expected})",
        key_id_hex(&key.id())
    )))
}

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
#[path = "store_rotation_tests.rs"]
mod rotation_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn key(digit: &str) -> StoreKey {
        StoreKey::parse_hex(&digit.repeat(32)).unwrap()
    }

    fn encrypted_under(key: &StoreKey) -> StoredEncryption {
        let mark = new_encrypted_store(key).unwrap().mark.unwrap();
        StoredEncryption {
            encrypted: true,
            key_id: Some(mark.key_id.to_vec()),
            verifier: Some(mark.verifier),
        }
    }

    fn plaintext() -> StoredEncryption {
        StoredEncryption::from_row("plaintext", None, None).unwrap()
    }

    #[test]
    fn a_new_store_with_a_key_asks_for_the_mark() {
        let resolved = resolve(&plaintext(), false, false, Some(&key("ab"))).unwrap();
        assert!(resolved.cipher.is_sealing());
        assert!(resolved.mark.is_some() && !resolved.convert);
    }

    #[test]
    fn plaintext_stays_plaintext_without_a_key() {
        for has_accounts in [false, true] {
            let resolved = resolve(&plaintext(), has_accounts, false, None).unwrap();
            assert!(!resolved.cipher.is_sealing() && resolved.mark.is_none());
            assert!(!resolved.convert);
        }
    }

    #[test]
    fn plaintext_with_accounts_and_a_key_converts_and_marks_the_key_first() {
        let resolved = resolve(&plaintext(), true, false, Some(&key("ab"))).unwrap();
        assert!(resolved.cipher.is_sealing());
        assert!(resolved.mark.is_some() && resolved.convert);
    }

    #[test]
    fn an_interrupted_conversion_resumes_with_its_key_only() {
        let stored = StoredEncryption {
            encrypted: false,
            ..encrypted_under(&key("ab"))
        };
        let resumed = resolve(&stored, true, true, Some(&key("ab"))).unwrap();
        assert!(resumed.convert && resumed.mark.is_none());
        let wrong = resolve(&stored, true, true, Some(&key("cd")))
            .err()
            .unwrap();
        assert!(wrong.to_string().contains("does not match"));
        let keyless = resolve(&stored, true, true, None)
            .err()
            .unwrap()
            .to_string();
        assert!(keyless.contains("interrupted") && keyless.contains("store_key_file"));
    }

    #[test]
    fn an_encrypted_store_with_progress_is_a_half_finished_decrypt() {
        let stored = encrypted_under(&key("ab"));
        for key in [None, Some(key("ab")), Some(key("cd"))] {
            let shown = resolve(&stored, true, true, key.as_ref())
                .err()
                .unwrap()
                .to_string();
            assert!(shown.contains("interrupted") && shown.contains("wamux store decrypt"));
        }
    }

    #[test]
    fn an_encrypted_store_needs_its_own_key() {
        let stored = encrypted_under(&key("ab"));
        let keyless = resolve(&stored, true, false, None)
            .err()
            .unwrap()
            .to_string();
        assert!(keyless.contains("store_key_file") && keyless.contains("encrypted"));
        let wrong = resolve(&stored, true, false, Some(&key("cd")))
            .err()
            .unwrap();
        assert!(wrong.to_string().contains("does not match"));
        let right = resolve(&stored, true, false, Some(&key("ab"))).unwrap();
        assert!(right.cipher.is_sealing() && right.mark.is_none() && !right.convert);
    }
}
