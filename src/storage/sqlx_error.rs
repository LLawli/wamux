//! Map `sqlx::Error` onto wacore's `StoreError` taxonomy. Engine-agnostic:
//! `sqlx::Error` is the same type for the Postgres and SQLite drivers.

use wacore::store::error::StoreError;

/// Convert a sqlx error into the store error the traits must return.
/// Connection-class failures map to `Connection`; everything else to `Database`.
/// (`RowNotFound` never reaches here: getters use `fetch_optional`.)
pub fn db(err: sqlx::Error) -> StoreError {
    match err {
        sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed | sqlx::Error::Io(_) => {
            StoreError::Connection(Box::new(err))
        }
        other => StoreError::Database(Box::new(other)),
    }
}
