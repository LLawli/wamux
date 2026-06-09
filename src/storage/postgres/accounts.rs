//! The wamux-specific `accounts` table: maps the canonical UUID and optional
//! `external_ref` to the integer `device_id` that scopes every store table.
//! This is NOT a wacore trait — it is wamux's own account registry persistence.

use sqlx::PgPool;
use uuid::Uuid;

use super::error_map::db;
use wacore::store::error::Result;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AccountRow {
    pub uuid: Uuid,
    pub external_ref: Option<String>,
    pub device_id: i32,
    pub push_name: Option<String>,
    pub created_at: i64,
}

/// CRUD over the `accounts` table.
#[derive(Clone)]
pub struct Accounts {
    pool: PgPool,
}

impl Accounts {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Insert a new account; `device_id` is assigned by the IDENTITY column.
    pub async fn create(&self, external_ref: Option<&str>) -> Result<AccountRow> {
        let uuid = Uuid::new_v4();
        let row: AccountRow = sqlx::query_as(
            "INSERT INTO accounts (uuid, external_ref)
             VALUES ($1, $2)
             RETURNING uuid, external_ref, device_id, push_name, created_at",
        )
        .bind(uuid)
        .bind(external_ref)
        .fetch_one(&self.pool)
        .await
        .map_err(db)?;
        Ok(row)
    }

    pub async fn get_by_uuid(&self, uuid: Uuid) -> Result<Option<AccountRow>> {
        let row: Option<AccountRow> = sqlx::query_as(
            "SELECT uuid, external_ref, device_id, push_name, created_at
             FROM accounts WHERE uuid = $1",
        )
        .bind(uuid)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row)
    }

    pub async fn get_by_external_ref(&self, external_ref: &str) -> Result<Option<AccountRow>> {
        let row: Option<AccountRow> = sqlx::query_as(
            "SELECT uuid, external_ref, device_id, push_name, created_at
             FROM accounts WHERE external_ref = $1",
        )
        .bind(external_ref)
        .fetch_optional(&self.pool)
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

    pub async fn set_push_name(&self, uuid: Uuid, push_name: &str) -> Result<()> {
        sqlx::query("UPDATE accounts SET push_name = $2 WHERE uuid = $1")
            .bind(uuid)
            .bind(push_name)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    /// Delete the account; ON DELETE CASCADE removes all scoped store rows.
    pub async fn delete(&self, uuid: Uuid) -> Result<bool> {
        let res = sqlx::query("DELETE FROM accounts WHERE uuid = $1")
            .bind(uuid)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(res.rows_affected() > 0)
    }
}
