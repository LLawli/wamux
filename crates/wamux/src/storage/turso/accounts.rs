//! The wamux-specific `accounts` table on turso (#106). `uuid` is hyphenated
//! TEXT, as on SQLite: bind `uuid.to_string()`, never the `Uuid` itself (a
//! blob would not parse back on a store SQLite also opens).

use ::turso::Row;
use uuid::Uuid;
use wacore::store::error::{Result, StoreError};

use super::TursoConn;
use super::exec::binds;
use super::row_values::{int, int32, opt_text, uuid_text};
use crate::storage::engine::AccountRow;
use crate::storage::statements::accounts::{DELETE_ACCOUNT, INSERT_ACCOUNT, LIST_ACCOUNTS};

/// `uuid, external_ref, device_id, push_name, created_at`, the column order of
/// every `accounts` SELECT and of the INSERT's RETURNING.
fn account_row(row: &Row) -> Result<AccountRow> {
    Ok(AccountRow {
        uuid: uuid_text(row, 0)?,
        external_ref: opt_text(row, 1)?,
        device_id: int32(row, 2)?,
        push_name: opt_text(row, 3)?,
        created_at: int(row, 4)?,
    })
}

/// `device_id` is assigned by AUTOINCREMENT. Goes through
/// `write_returning_optional`: a write with RETURNING (see `exec`).
pub(super) async fn create_account(
    conn: &TursoConn,
    external_ref: Option<&str>,
) -> Result<AccountRow> {
    let uuid = Uuid::new_v4().to_string();
    let row = conn
        .write_returning_optional(INSERT_ACCOUNT, binds![uuid, external_ref])
        .await?;
    let row =
        row.ok_or_else(|| StoreError::Validation("the account insert returned no row".into()))?;
    account_row(&row)
}

pub(super) async fn list_accounts(conn: &TursoConn) -> Result<Vec<AccountRow>> {
    let rows = conn.fetch_all(LIST_ACCOUNTS, binds![]).await?;
    rows.iter().map(account_row).collect()
}

/// ON DELETE CASCADE removes the scoped rows, because `foreign_keys` is on for
/// this connection (`connection::apply_pragmas`).
pub(super) async fn delete_account(conn: &TursoConn, uuid: Uuid) -> Result<bool> {
    let deleted = conn
        .execute(DELETE_ACCOUNT, binds![uuid.to_string()])
        .await?;
    Ok(deleted > 0)
}
