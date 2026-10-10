//! The statements of the store conversion and of `wamux store decrypt` (#165).
//! The per-column ones are generated from `sealed_columns()`, the one list of
//! the 13 secret columns, so no table or column is typed twice. Every name in
//! them is a constant of that list, never request input.

use crate::storage::sealed_columns::{KeyColumn, SealedColumn};

pub const SELECT_ACCOUNT_IDS: &str = "SELECT device_id FROM accounts ORDER BY device_id";

pub const SELECT_PROGRESS: &str = "SELECT device_id FROM store_conversion_progress";

pub const INSERT_PROGRESS: &str = "INSERT INTO store_conversion_progress (device_id) VALUES ($1)";

pub const CLEAR_PROGRESS: &str = "DELETE FROM store_conversion_progress";

/// The last step of a conversion, in the transaction of the last account.
pub const FINISH_SEALING: &str =
    "UPDATE store_encryption SET state = 'encrypted' WHERE id = 1 AND state = 'plaintext'";

/// The last step of a decrypt: the store names no key any more.
pub const FINISH_OPENING: &str = "UPDATE store_encryption
     SET state = 'plaintext', key_id = NULL, verifier = NULL
     WHERE id = 1 AND state = 'encrypted'";

/// SQLite and Turso: rewrite the file without the free pages that still hold
/// the old plaintext. In WAL mode this is not enough alone, see `WAL_TRUNCATE`.
pub const VACUUM_FILE: &str = "VACUUM";

/// SQLite and Turso: fold the WAL into the file and truncate it, which drops
/// the plaintext pages the WAL kept. Measured: only VACUUM followed by this
/// leaves no plaintext in the file or the WAL.
pub const WAL_TRUNCATE: &str = "PRAGMA wal_checkpoint(TRUNCATE)";

fn key_names(column: &SealedColumn) -> Vec<&'static str> {
    column
        .key
        .iter()
        .map(|key| match key {
            KeyColumn::Text(name) | KeyColumn::Id(name) | KeyColumn::Bytes(name) => *name,
        })
        .collect()
}

/// Every row of one account in this column: the key columns, then the blob.
pub fn select_rows(column: &SealedColumn) -> String {
    let mut names = key_names(column);
    names.push(column.column);
    format!(
        "SELECT {} FROM {} WHERE device_id = $1",
        names.join(", "),
        column.table
    )
}

/// Replace the blob of one row: `$1` the blob, `$2` the account, then one
/// bind per key column in list order.
pub fn update_row(column: &SealedColumn) -> String {
    let mut sql = format!(
        "UPDATE {} SET {} = $1 WHERE device_id = $2",
        column.table, column.column
    );
    for (position, name) in key_names(column).into_iter().enumerate() {
        sql.push_str(&format!(" AND {name} = ${}", position + 3));
    }
    sql
}

/// Postgres only: rewrite one table so the dead tuples that hold the old
/// plaintext are gone. Cannot run inside a transaction.
pub fn vacuum_full(column: &SealedColumn) -> String {
    format!("VACUUM FULL {}", column.table)
}
