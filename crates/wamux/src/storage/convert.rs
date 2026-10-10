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

/// One blob of `column`, turned the way `direction` says. The row key comes
/// from `sealed_columns`, the same rule the stores seal and open with. An empty
/// blob stays empty (the cipher guarantees it).
pub(crate) fn turn_blob(
    cipher: &BlobCipher,
    direction: Direction,
    device_id: i32,
    column: &SealedColumn,
    parts: &[RowPart],
    blob: &[u8],
) -> StoreResult<Vec<u8>> {
    let row = row_key(column, parts);
    match direction {
        Direction::Seal => cipher.seal_at(device_id, column.table, column.column, &row, blob),
        Direction::Open => cipher.open_at(device_id, column.table, column.column, &row, blob),
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
        let sealed = turn_blob(&cipher, Direction::Seal, 4, &column, &parts, b"secret").unwrap();
        assert_ne!(sealed, b"secret");
        let opened = turn_blob(&cipher, Direction::Open, 4, &column, &parts, &sealed).unwrap();
        assert_eq!(opened, b"secret");
        let empty = turn_blob(&cipher, Direction::Seal, 4, &column, &parts, b"").unwrap();
        assert!(empty.is_empty());
    }
}
