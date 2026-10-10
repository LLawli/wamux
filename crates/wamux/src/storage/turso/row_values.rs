//! Reading columns off a `turso::Row` (#106). turso has no `FromValue` for
//! `Vec<u8>` (blobs come out of `get_value` as `Value::Blob`), and reading NULL
//! as a plain `i64` is an error, so each shape has its own strict reader. A
//! column of the wrong type is a statement or schema bug: it fails loudly
//! rather than coercing.

use ::turso::{Row, Value};
use uuid::Uuid;
use wacore::store::error::{Result as StoreResult, StoreError};

use super::turso_error::{ColumnTypeError, db};

fn wrong_type(column: usize, expected: &'static str, got: &Value) -> StoreError {
    StoreError::Database(Box::new(ColumnTypeError {
        column,
        expected,
        got: format!("{got:?}"),
    }))
}

pub(super) fn blob(row: &Row, column: usize) -> StoreResult<Vec<u8>> {
    match row.get_value(column).map_err(db)? {
        Value::Blob(bytes) => Ok(bytes),
        other => Err(wrong_type(column, "a blob", &other)),
    }
}

pub(super) fn opt_blob(row: &Row, column: usize) -> StoreResult<Option<Vec<u8>>> {
    match row.get_value(column).map_err(db)? {
        Value::Null => Ok(None),
        Value::Blob(bytes) => Ok(Some(bytes)),
        other => Err(wrong_type(column, "a blob or NULL", &other)),
    }
}

pub(super) fn text(row: &Row, column: usize) -> StoreResult<String> {
    match row.get_value(column).map_err(db)? {
        Value::Text(text) => Ok(text),
        other => Err(wrong_type(column, "text", &other)),
    }
}

pub(super) fn opt_text(row: &Row, column: usize) -> StoreResult<Option<String>> {
    match row.get_value(column).map_err(db)? {
        Value::Null => Ok(None),
        Value::Text(text) => Ok(Some(text)),
        other => Err(wrong_type(column, "text or NULL", &other)),
    }
}

pub(super) fn int(row: &Row, column: usize) -> StoreResult<i64> {
    match row.get_value(column).map_err(db)? {
        Value::Integer(n) => Ok(n),
        other => Err(wrong_type(column, "an integer", &other)),
    }
}

pub(super) fn opt_int(row: &Row, column: usize) -> StoreResult<Option<i64>> {
    match row.get_value(column).map_err(db)? {
        Value::Null => Ok(None),
        Value::Integer(n) => Ok(Some(n)),
        other => Err(wrong_type(column, "an integer or NULL", &other)),
    }
}

/// An `INTEGER` column the schema keeps in `i32` range (`device_id`, `raw_id`).
pub(super) fn int32(row: &Row, column: usize) -> StoreResult<i32> {
    let wide = int(row, column)?;
    i32::try_from(wide).map_err(|_| wrong_type(column, "a 32-bit integer", &Value::Integer(wide)))
}

pub(super) fn opt_int32(row: &Row, column: usize) -> StoreResult<Option<i32>> {
    opt_int(row, column)?
        .map(|wide| {
            i32::try_from(wide)
                .map_err(|_| wrong_type(column, "a 32-bit integer", &Value::Integer(wide)))
        })
        .transpose()
}

/// `accounts.uuid` is hyphenated TEXT here, never a blob.
pub(super) fn uuid_text(row: &Row, column: usize) -> StoreResult<Uuid> {
    let raw = text(row, column)?;
    Uuid::parse_str(&raw).map_err(|e| StoreError::Serialization(Box::new(e)))
}

/// The first column of the first row as a blob: the one-value lookups.
pub(super) fn first_blob(row: Option<Row>) -> StoreResult<Option<Vec<u8>>> {
    row.map(|row| blob(&row, 0)).transpose()
}
