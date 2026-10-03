//! The SQL engine family (#65): Postgres and SQLite through sqlx, every
//! statement written once.
//!
//! `SqlBackend` implements wacore's store traits (`SignalStore`, `AppSyncStore`,
//! `ProtocolStore`, `DeviceStore`, plus `MsgSecretStore`) on a `SqlPool`; since
//! `Backend` is a blanket impl over them, that makes it a `Backend`. Statements
//! are `$N` strings that run on both drivers; `macros` expands each one per
//! driver. Only four places write a string per driver, because the dialects
//! have no common form there: `accounts` (UUID vs TEXT), `prekeys_sql`
//! (array bind vs a loop), `maintenance_sql` (SQLite upkeep vs Postgres
//! autovacuum) and the `blob_format` row lock in `bincode_upgrade`.
//!
//! Multi-tenancy: one shared pool, one `SqlBackend` per account, each carrying
//! the integer `device_id` that scopes every row. A future engine that is not
//! sqlx (#106) is a sibling family behind `StorageEngine`, not a variant here.

#[macro_use]
mod macros;

mod accounts;
mod app_sync_sql;
mod app_sync_store;
mod batch_sql;
mod bincode_upgrade;
mod connect;
mod device_store;
mod maintenance_sql;
mod msg_secret_store;
mod prekeys_sql;
mod protocol_batch_sql;
mod protocol_rows;
mod protocol_store;
mod signal_sql;
mod signal_store;
mod tc_token_sql;
mod transaction;

use std::sync::Arc;

use sqlx::{PgPool, SqlitePool};
use uuid::Uuid;
use wacore::store::error::{Result as StoreResult, StoreError};
use wacore::store::traits::Backend;

use crate::storage::engine::{AccountRow, StorageEngine};

pub use connect::{connect_postgres, connect_sqlite};
pub(crate) use transaction::SqlTx;

/// The one pool a `SqlStore` runs on. Each method matches on it to pick the
/// driver; the statement text is the same in both arms.
#[derive(Clone)]
pub enum SqlPool {
    Pg(PgPool),
    Sqlite(SqlitePool),
}

/// Embedded Postgres migrations (compiled in; no DB needed at build time).
pub static PG_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Embedded SQLite migrations: a separate tree from `./migrations` (same
/// tables, SQLite dialect), same numbering (#66).
pub static SQLITE_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations_sqlite");

/// Apply pending SQL migrations, then the blob-format conversion SQL cannot do
/// (`storage::bincode_upgrade`, #31). Every path to a usable pool runs this, so
/// no caller can reach a store whose blobs are still bincode.
pub async fn run_migrations(pool: &SqlPool) -> Result<(), StoreError> {
    let migrated = match pool {
        SqlPool::Pg(pool) => PG_MIGRATOR.run(pool).await,
        SqlPool::Sqlite(pool) => SQLITE_MIGRATOR.run(pool).await,
    };
    migrated.map_err(|e| StoreError::Migration(Box::new(e)))?;
    bincode_upgrade::upgrade_bincode_blobs(pool)
        .await
        .map_err(|e| StoreError::Migration(Box::new(e)))
}

/// The SQL engine: one pool, the `accounts` table, one `SqlBackend` per account.
#[derive(Clone)]
pub struct SqlStore {
    pool: SqlPool,
}

impl SqlStore {
    /// Connect to Postgres and apply pending migrations.
    pub async fn open_postgres(database_url: &str, max_connections: u32) -> StoreResult<Self> {
        let pool = connect_postgres(database_url, max_connections)
            .await
            .map_err(|e| StoreError::Connection(Box::new(e)))?;
        Self::migrated(SqlPool::Pg(pool)).await
    }

    /// Open (creating if absent) the SQLite file and apply pending migrations.
    pub async fn open_sqlite(database_url: &str) -> StoreResult<Self> {
        let pool = connect_sqlite(database_url)
            .await
            .map_err(|e| StoreError::Connection(Box::new(e)))?;
        Self::migrated(SqlPool::Sqlite(pool)).await
    }

    async fn migrated(pool: SqlPool) -> StoreResult<Self> {
        run_migrations(&pool).await?;
        Ok(Self::from_pool(pool))
    }

    /// Wrap an already-connected, already-migrated pool (bins and test
    /// harnesses that build the pool themselves).
    pub fn from_pool(pool: SqlPool) -> Self {
        Self { pool }
    }

    /// Raw pool access for callers that issue their own SQL (the stress bins).
    /// Not part of `StorageEngine`: no engine-agnostic caller may assume a SQL
    /// pool.
    pub fn pool(&self) -> &SqlPool {
        &self.pool
    }
}

#[async_trait::async_trait]
impl StorageEngine for SqlStore {
    async fn create_account(&self, external_ref: Option<&str>) -> StoreResult<AccountRow> {
        accounts::create_account(&self.pool, external_ref).await
    }

    async fn list_accounts(&self) -> StoreResult<Vec<AccountRow>> {
        accounts::list_accounts(&self.pool).await
    }

    async fn delete_account(&self, uuid: Uuid) -> StoreResult<bool> {
        accounts::delete_account(&self.pool, uuid).await
    }

    fn device_backend(&self, device_id: i32) -> Arc<dyn Backend> {
        Arc::new(SqlBackend::new(self.pool.clone(), device_id))
    }

    async fn ping_storage(&self) -> bool {
        // A trivial round-trip: proves the pool can hand out a live connection,
        // which is exactly what readiness means here.
        on_sql_pool!(&self.pool, |conn| {
            sqlx::query("SELECT 1").execute(conn).await.is_ok()
        })
    }
}

/// wacore's storage backend bound to one account's `device_id`.
#[derive(Clone)]
pub struct SqlBackend {
    pub(crate) pool: SqlPool,
    pub(crate) device_id: i32,
}

impl SqlBackend {
    pub fn new(pool: SqlPool, device_id: i32) -> Self {
        Self { pool, device_id }
    }

    pub fn device_id(&self) -> i32 {
        self.device_id
    }
}
