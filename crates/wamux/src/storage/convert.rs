//! What both engine families share in a store conversion (#165): which way a
//! blob is turned, how one blob is turned, and which accounts are left. The
//! engines own the transactions; this file owns no I/O.

use wacore::store::error::Result as StoreResult;

use super::blob_cipher::BlobCipher;
use super::sealed_columns::{RowPart, SealedColumn, row_key};
use super::statements::convert::{FINISH_OPENING, FINISH_SEALING};

/// Which way a conversion turns the secret columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direction {
    /// Plaintext to sealed blobs: turning a key on over an existing store.
    Seal,
    /// Sealed blobs to plaintext: `wamux store decrypt`.
    Open,
}

impl Direction {
    /// The statement that ends this direction, run in the transaction of the
    /// last account so the state flips exactly when the last blob does.
    pub(crate) fn finish_statement(self) -> &'static str {
        match self {
            Direction::Seal => FINISH_SEALING,
            Direction::Open => FINISH_OPENING,
        }
    }

    pub(crate) fn verb(self) -> &'static str {
        match self {
            Direction::Seal => "sealing",
            Direction::Open => "opening",
        }
    }
}

/// How one blob is turned: the cipher(s) and the way. Conversion and decrypt
/// use one cipher (`Direction`); the rotation of #166 opens with one key and
/// seals with another, in the same pass over the same columns.
pub(crate) enum Turn<'a> {
    Seal(&'a BlobCipher),
    Open(&'a BlobCipher),
    Rotate {
        from: &'a BlobCipher,
        to: &'a BlobCipher,
    },
}

impl<'a> Turn<'a> {
    /// The turn a conversion run of `direction` does with `cipher`.
    pub(crate) fn of(direction: Direction, cipher: &'a BlobCipher) -> Self {
        match direction {
            Direction::Seal => Turn::Seal(cipher),
            Direction::Open => Turn::Open(cipher),
        }
    }
}

/// One blob of `column`, turned the way `turn` says. The row key comes from
/// `sealed_columns`, the same rule the stores seal and open with, and a
/// rotation uses that one row key to open and to seal. An empty blob stays
/// empty (the cipher guarantees it).
pub(crate) fn turn_blob(
    turn: &Turn<'_>,
    device_id: i32,
    column: &SealedColumn,
    parts: &[RowPart],
    blob: &[u8],
) -> StoreResult<Vec<u8>> {
    let row = row_key(column, parts);
    let (table, name) = (column.table, column.column);
    match turn {
        Turn::Seal(cipher) => cipher.seal_at(device_id, table, name, &row, blob),
        Turn::Open(cipher) => cipher.open_at(device_id, table, name, &row, blob),
        Turn::Rotate { from, to } => {
            let plain = from.open_at(device_id, table, name, &row, blob)?;
            to.seal_at(device_id, table, name, &row, &plain)
        }
    }
}

/// The accounts a run still has to do, in `device_id` order: every account
/// minus the ones the progress table already names.
pub(crate) fn accounts_left(mut all: Vec<i32>, done: &[i32]) -> Vec<i32> {
    all.sort_unstable();
    all.retain(|device_id| !done.contains(device_id));
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::sealed_columns::sealed_columns;
    use crate::storage::store_key::StoreKey;

    #[test]
    fn accounts_left_skips_the_done_ones_in_order() {
        assert_eq!(accounts_left(vec![5, 2, 9, 3], &[3, 9]), vec![2, 5]);
        assert!(accounts_left(vec![1], &[1]).is_empty());
    }

    #[test]
    fn a_blob_sealed_then_opened_is_the_original_and_empty_stays_empty() {
        let key = StoreKey::parse_hex(&"ab".repeat(32)).unwrap();
        let cipher = BlobCipher::sealing(&key);
        let column = sealed_columns()[1];
        let parts = [RowPart::Text("alice@s.whatsapp.net".into())];
        let sealed = turn_blob(&Turn::Seal(&cipher), 4, &column, &parts, b"secret").unwrap();
        assert_ne!(sealed, b"secret");
        let opened = turn_blob(&Turn::Open(&cipher), 4, &column, &parts, &sealed).unwrap();
        assert_eq!(opened, b"secret");
        let empty = turn_blob(&Turn::Seal(&cipher), 4, &column, &parts, b"").unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn a_rotated_blob_opens_with_the_new_cipher_only() {
        let old = BlobCipher::sealing(&StoreKey::parse_hex(&"ab".repeat(32)).unwrap());
        let new = BlobCipher::sealing(&StoreKey::parse_hex(&"cd".repeat(32)).unwrap());
        let column = sealed_columns()[1];
        let parts = [RowPart::Text("alice@s.whatsapp.net".into())];
        let sealed = turn_blob(&Turn::Seal(&old), 4, &column, &parts, b"secret").unwrap();
        let rotate = Turn::Rotate {
            from: &old,
            to: &new,
        };
        let rotated = turn_blob(&rotate, 4, &column, &parts, &sealed).unwrap();
        let opened = turn_blob(&Turn::Open(&new), 4, &column, &parts, &rotated).unwrap();
        assert_eq!(opened, b"secret");
        assert!(turn_blob(&Turn::Open(&old), 4, &column, &parts, &rotated).is_err());
    }
}
