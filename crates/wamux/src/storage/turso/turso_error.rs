//! Map `turso::Error` onto wacore's `StoreError` taxonomy, the way
//! `storage::sqlx_error` does for sqlx (#106).

use wacore::store::error::StoreError;

/// A column that is not the type the statement promised: a schema or a
/// statement bug, never user data, so it reads as a database error.
#[derive(Debug, thiserror::Error)]
#[error("column {column} is {got}, expected {expected}")]
pub struct ColumnTypeError {
    pub column: usize,
    pub expected: &'static str,
    pub got: String,
}

/// Busy, busy-snapshot and I/O failures are the connection's (the file is
/// locked or unreachable, a retry layer may care); everything else is the
/// statement's.
pub fn db(err: ::turso::Error) -> StoreError {
    match err {
        ::turso::Error::Busy(_) | ::turso::Error::BusySnapshot(_) | ::turso::Error::IoError(..) => {
            StoreError::Connection(Box::new(err))
        }
        other => StoreError::Database(Box::new(other)),
    }
}

/// Only "someone else holds the lock", which the checkpoint pass tolerates.
pub fn is_busy(err: &::turso::Error) -> bool {
    matches!(
        err,
        ::turso::Error::Busy(_) | ::turso::Error::BusySnapshot(_)
    )
}
