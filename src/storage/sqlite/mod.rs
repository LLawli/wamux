//! SQLite implementation of wamux's storage. The sibling of `postgres/`: same
//! schema, same byte formats, same statements modulo dialect.
//!
//! For a single-host deployment this removes the Postgres process entirely; the
//! trade-off is write concurrency (see `connect` below), so many simultaneously
//! connected accounts still want Postgres.

mod accounts;
mod app_sync_store;
mod device_store;
mod protocol_store;
mod signal_store;

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use wacore::store::error::{Result as StoreResult, StoreError};
use wacore::store::traits::Backend;

use crate::storage::engine::{AccountRow, StorageEngine};
use accounts::SqliteAccounts;

/// Embedded migrations (compiled in; no DB needed at build time). Separate tree
/// from `./migrations`: same tables, SQLite dialect.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations_sqlite");

/// A storage backend bound to a single account's `device_id`.
#[derive(Clone)]
pub struct SqliteBackend {
    pub(crate) pool: SqlitePool,
    pub(crate) device_id: i32,
}

impl SqliteBackend {
    pub fn new(pool: SqlitePool, device_id: i32) -> Self {
        Self { pool, device_id }
    }

    pub fn device_id(&self) -> i32 {
        self.device_id
    }
}

/// Open the database file, applying the reference implementation's pragmas.
///
/// `max_connections(1)` is deliberate and not a leftover. SQLite allows one
/// writer at a time, and sqlx opens transactions as BEGIN DEFERRED: two pooled
/// connections upgrading to a write inside a transaction can hit SQLITE_BUSY
/// *without* honoring `busy_timeout`, which would surface as a random store
/// error under multi-account load. A single connection makes the process
/// serialize its own writes instead, which is exactly what the whatsapp-rust
/// sqlite reference achieves with its 1-permit semaphore.
///
/// `foreign_keys` is likewise not cosmetic: SQLite defaults it OFF, and every
/// store table hangs off `accounts(device_id) ON DELETE CASCADE`. Without it,
/// deleting an account would silently orphan all of its Signal rows.
pub async fn connect(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(30))
        .foreign_keys(true);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
}

/// Apply pending migrations.
pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::migrate::MigrateError> {
    MIGRATOR.run(pool).await
}

/// The SQLite engine: one file, one connection, N `SqliteBackend`s.
#[derive(Clone)]
pub struct SqliteStorage {
    pool: SqlitePool,
    accounts: SqliteAccounts,
}

impl SqliteStorage {
    /// Open (creating if absent) the database file and apply pending migrations.
    pub async fn open(database_url: &str) -> Result<Self, StoreError> {
        let pool = connect(database_url)
            .await
            .map_err(|e| StoreError::Connection(Box::new(e)))?;
        run_migrations(&pool)
            .await
            .map_err(|e| StoreError::Migration(Box::new(e)))?;
        Ok(Self::from_pool(pool))
    }

    /// Wrap an already-connected, already-migrated pool (test harnesses).
    pub fn from_pool(pool: SqlitePool) -> Self {
        let accounts = SqliteAccounts::new(pool.clone());
        Self { pool, accounts }
    }

    /// Raw pool access, for callers that issue their own SQL. Not part of
    /// `StorageEngine`: no engine-agnostic caller may assume a SQLite pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

#[async_trait::async_trait]
impl StorageEngine for SqliteStorage {
    async fn create_account(&self, external_ref: Option<&str>) -> StoreResult<AccountRow> {
        self.accounts.create(external_ref).await
    }

    async fn list_accounts(&self) -> StoreResult<Vec<AccountRow>> {
        self.accounts.list().await
    }

    async fn delete_account(&self, uuid: uuid::Uuid) -> StoreResult<bool> {
        self.accounts.delete(uuid).await
    }

    fn device_backend(&self, device_id: i32) -> Arc<dyn Backend> {
        Arc::new(SqliteBackend::new(self.pool.clone(), device_id))
    }

    async fn ping_storage(&self) -> bool {
        sqlx::query("SELECT 1").execute(&self.pool).await.is_ok()
    }
}
