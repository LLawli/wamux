//! The wamux-specific `accounts` table: maps the canonical UUID and optional
//! `external_ref` to the integer `device_id` that scopes every store table.
//! This is NOT a wacore trait, it is wamux's own account registry persistence.
//!
//! The statement text is shared, but `uuid` is a UUID column on Postgres and
//! TEXT on SQLite (`migrations_sqlite/0001_initial.sql` also makes `device_id`
//! the primary key there), so the bind and the decode are per driver. Never
//! bind a `Uuid` on SQLite: sqlx-sqlite writes it as a 16-byte BLOB, which the
//! TEXT reads of every existing store would then fail to parse.

use sqlx::Row;
use sqlx::postgres::PgRow;
use sqlx::sqlite::SqliteRow;
use uuid::Uuid;
use wacore::store::error::Result;

use super::SqlPool;
use crate::storage::engine::AccountRow;
use crate::storage::sqlx_error::db;
use crate::storage::statements::accounts::{DELETE_ACCOUNT, INSERT_ACCOUNT, LIST_ACCOUNTS};

/// Decode a Postgres row into the neutral `AccountRow`: `uuid` from UUID,
/// `created_at` from BIGINT.
impl sqlx::FromRow<'_, PgRow> for AccountRow {
    fn from_row(row: &PgRow) -> sqlx::Result<Self> {
        Ok(Self {
            uuid: row.try_get("uuid")?,
            external_ref: row.try_get("external_ref")?,
            device_id: row.try_get("device_id")?,
            push_name: row.try_get("push_name")?,
            created_at: row.try_get("created_at")?,
        })
    }
}

/// Decode a SQLite row into the neutral `AccountRow`. `uuid` round-trips as
/// hyphenated TEXT, so it is parsed here.
impl sqlx::FromRow<'_, SqliteRow> for AccountRow {
    fn from_row(row: &SqliteRow) -> sqlx::Result<Self> {
        let raw: String = row.try_get("uuid")?;
        let uuid = Uuid::parse_str(&raw).map_err(|e| sqlx::Error::ColumnDecode {
            index: "uuid".to_string(),
            source: Box::new(e),
        })?;
        Ok(Self {
            uuid,
            external_ref: row.try_get("external_ref")?,
            device_id: row.try_get("device_id")?,
            push_name: row.try_get("push_name")?,
            created_at: row.try_get("created_at")?,
        })
    }
}

/// Insert a new account; `device_id` is assigned by the IDENTITY column
/// (Postgres) or AUTOINCREMENT (SQLite).
pub(super) async fn create_account(
    pool: &SqlPool,
    external_ref: Option<&str>,
) -> Result<AccountRow> {
    let uuid = Uuid::new_v4();
    let row: sqlx::Result<AccountRow> = match pool {
        SqlPool::Pg(pool) => {
            sqlx::query_as(INSERT_ACCOUNT)
                .bind(uuid)
                .bind(external_ref)
                .fetch_one(pool)
                .await
        }
        SqlPool::Sqlite(pool) => {
            sqlx::query_as(INSERT_ACCOUNT)
                .bind(uuid.to_string())
                .bind(external_ref)
                .fetch_one(pool)
                .await
        }
    };
    row.map_err(db)
}

/// Every account, ordered by `device_id`. No bind, so one body serves both
/// drivers; the row type picks the decoder.
pub(super) async fn list_accounts(pool: &SqlPool) -> Result<Vec<AccountRow>> {
    row_all_sql!(AccountRow, pool, LIST_ACCOUNTS)
}

/// Delete the account; ON DELETE CASCADE removes all scoped store rows (on
/// SQLite only because `connect_sqlite` turns `foreign_keys` on).
pub(super) async fn delete_account(pool: &SqlPool, uuid: Uuid) -> Result<bool> {
    let res: sqlx::Result<u64> = match pool {
        SqlPool::Pg(pool) => sqlx::query(DELETE_ACCOUNT)
            .bind(uuid)
            .execute(pool)
            .await
            .map(|done| done.rows_affected()),
        SqlPool::Sqlite(pool) => sqlx::query(DELETE_ACCOUNT)
            .bind(uuid.to_string())
            .execute(pool)
            .await
            .map(|done| done.rows_affected()),
    };
    Ok(res.map_err(db)? > 0)
}
