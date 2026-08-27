//! The wamux-specific `accounts` table, SQLite dialect. Sibling of
//! `postgres/accounts.rs`; see `migrations_sqlite/0001_initial.sql` for why
//! `device_id` is the primary key here and `uuid` is TEXT.

use sqlx::Row;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteRow;
use uuid::Uuid;

use crate::storage::engine::AccountRow;
use crate::storage::sqlx_error::db;
use wacore::store::error::Result;

/// Decode a SQLite row into the neutral `AccountRow`. `uuid` round-trips as
/// hyphenated TEXT (Postgres uses a native UUID column), so it is parsed here.
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

/// CRUD over the `accounts` table.
#[derive(Clone)]
pub struct SqliteAccounts {
    pool: SqlitePool,
}

impl SqliteAccounts {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Insert a new account; `device_id` is assigned by AUTOINCREMENT.
    pub async fn create(&self, external_ref: Option<&str>) -> Result<AccountRow> {
        let uuid = Uuid::new_v4();
        let row: AccountRow = sqlx::query_as(
            "INSERT INTO accounts (uuid, external_ref)
             VALUES (?, ?)
             RETURNING uuid, external_ref, device_id, push_name, created_at",
        )
        .bind(uuid.to_string())
        .bind(external_ref)
        .fetch_one(&self.pool)
        .await
        .map_err(db)?;
        Ok(row)
    }

    pub async fn list(&self) -> Result<Vec<AccountRow>> {
        let rows: Vec<AccountRow> = sqlx::query_as(
            "SELECT uuid, external_ref, device_id, push_name, created_at
             FROM accounts ORDER BY device_id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(rows)
    }

    /// Delete the account; ON DELETE CASCADE removes all scoped store rows
    /// (only because `connect` turns `foreign_keys` on).
    pub async fn delete(&self, uuid: Uuid) -> Result<bool> {
        let res = sqlx::query("DELETE FROM accounts WHERE uuid = ?")
            .bind(uuid.to_string())
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(res.rows_affected() > 0)
    }
}
