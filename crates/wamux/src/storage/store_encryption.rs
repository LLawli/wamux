//! Deciding, when a store opens, whether its secret columns are sealed and
//! with what (#164). Pure: the engines read the `store_encryption` row and the
//! account count, call `resolve`, and write the mark it asks for.
//!
//! The decision table:
//!
//! | state     | accounts | key   | result                                  |
//! |-----------|----------|-------|-----------------------------------------|
//! | plaintext | none     | no    | plaintext, passthrough                  |
//! | plaintext | none     | yes   | NEW store: mark encrypted, sealing      |
//! | plaintext | some     | no    | plaintext, passthrough                  |
//! | plaintext | some     | yes   | refused, converting is #165             |
//! | encrypted | any      | no    | refused                                 |
//! | encrypted | any      | wrong | refused (verifier does not open)        |
//! | encrypted | any      | right | sealing                                 |
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
    key: Option<&StoreKey>,
) -> StoreResult<Resolution> {
    match (stored.encrypted, key) {
        (false, None) => Ok(passthrough()),
        (false, Some(_)) if has_accounts => Err(StoreError::InvalidConfig(
            "store_key_file is set but this store is not encrypted yet and already has accounts: \
             a key cannot be turned on over existing data until the store is converted \
             (convert is not available in this version; use a new store, or remove store_key_file)"
                .into(),
        )),
        (false, Some(key)) => new_encrypted_store(key),
        (true, None) => Err(StoreError::InvalidConfig(
            "this store is encrypted: set store_key_file (WAMUX_STORE_KEY_FILE) to the key file \
             it was created with"
                .into(),
        )),
        (true, Some(key)) => verify_key(stored, key),
    }
}

fn passthrough() -> Resolution {
    Resolution {
        cipher: BlobCipher::passthrough(),
        mark: None,
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
    })
}

fn verify_key(stored: &StoredEncryption, key: &StoreKey) -> StoreResult<Resolution> {
    let cipher = BlobCipher::sealing(key);
    let opened = stored
        .verifier
        .as_deref()
        .map(|verifier| cipher.open(&VERIFIER_CONTEXT, verifier));
    if matches!(&opened, Some(Ok(plain)) if plain == VERIFIER_PLAINTEXT) {
        return Ok(Resolution { cipher, mark: None });
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
        let resolved = resolve(&plaintext(), false, Some(&key("ab"))).unwrap();
        assert!(resolved.cipher.is_sealing());
        assert!(resolved.mark.is_some());
    }

    #[test]
    fn plaintext_stays_plaintext_without_a_key() {
        for has_accounts in [false, true] {
            let resolved = resolve(&plaintext(), has_accounts, None).unwrap();
            assert!(!resolved.cipher.is_sealing() && resolved.mark.is_none());
        }
    }

    #[test]
    fn plaintext_with_accounts_refuses_a_key() {
        let error = resolve(&plaintext(), true, Some(&key("ab"))).err().unwrap();
        let shown = error.to_string();
        assert!(shown.contains("not encrypted yet") && shown.contains("convert"));
    }

    #[test]
    fn an_encrypted_store_needs_its_own_key() {
        let stored = encrypted_under(&key("ab"));
        let keyless = resolve(&stored, true, None).err().unwrap().to_string();
        assert!(keyless.contains("store_key_file") && keyless.contains("encrypted"));
        let wrong = resolve(&stored, true, Some(&key("cd"))).err().unwrap();
        assert!(wrong.to_string().contains("does not match"));
        let right = resolve(&stored, true, Some(&key("ab"))).unwrap();
        assert!(right.cipher.is_sealing() && right.mark.is_none());
    }
}
