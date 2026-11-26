//! Map `sqlx::Error` onto wacore's `StoreError` taxonomy. Engine-agnostic:
//! `sqlx::Error` is the same type for the Postgres and SQLite drivers.

use wacore::store::error::StoreError;

use super::engine::CreateAccountError;

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

/// The `external_ref` a failed accounts INSERT collided on, if that is what
/// failed. Classified by error kind, never by message text. Only with a
/// `Some` ref: a NULL never collides, and `uuid` (v4) and `device_id`
/// (engine-assigned) cannot, so a violation on a `None` ref is not about the
/// ref and stays a store error (#269).
pub fn taken_external_ref(err: &sqlx::Error, external_ref: Option<&str>) -> Option<String> {
    let taken = err
        .as_database_error()
        .is_some_and(|e| e.is_unique_violation());
    external_ref.filter(|_| taken).map(str::to_owned)
}

/// Turn a failed accounts INSERT into the typed error `create_account` returns.
pub fn create_account_error(err: sqlx::Error, external_ref: Option<&str>) -> CreateAccountError {
    match taken_external_ref(&err, external_ref) {
        Some(taken) => CreateAccountError::ExternalRefTaken(taken),
        None => CreateAccountError::Store(db(err)),
    }
}
