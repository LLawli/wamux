//! Postgres implementation of wacore's storage backend.
//!
//! `wacore::store::traits::Backend` is a blanket impl over the four domain
//! traits (`SignalStore`, `AppSyncStore`, `ProtocolStore`, `DeviceStore`), so
//! implementing all four on `PgBackend` makes it a `Backend`.
//!
//! Multi-tenancy: one shared `PgPool`, one `PgBackend` instance per account,
//! each carrying the integer `device_id` that scopes every row. Byte formats
//! match the sqlite reference exactly (see docs/crate-notes/sqlite-reference.md).

mod accounts;
mod app_sync_store;
mod device_store;
mod protocol_store;
mod signal_store;

pub use accounts::Accounts;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use wacore::store::error::StoreError;
use wacore::store::traits::Backend;

use crate::storage::blob_codec::{bincode_decode, bincode_encode, now_secs};

use crate::storage::engine::{AccountRow, StorageEngine};

/// Embedded migrations (compiled in; no DB needed at build time).
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// A storage backend bound to a single account's `device_id`.
#[derive(Clone)]
pub struct PgBackend {
    pub(crate) pool: PgPool,
    pub(crate) device_id: i32,
}

impl PgBackend {
    pub fn new(pool: PgPool, device_id: i32) -> Self {
        Self { pool, device_id }
    }

    pub fn device_id(&self) -> i32 {
        self.device_id
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

/// Open a pooled connection to Postgres.
pub async fn connect(database_url: &str, max_connections: u32) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(database_url)
        .await
}

/// Apply pending migrations.
pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    MIGRATOR.run(pool).await
}

/// The Postgres engine: one shared pool, the `accounts` table, and a
/// `PgBackend` minted per account. This is the `StorageEngine` half; `PgBackend`
/// stays the per-account wacore `Backend`.
#[derive(Clone)]
pub struct PgStorage {
    pool: PgPool,
    accounts: Accounts,
}

impl PgStorage {
    /// Connect to Postgres and apply pending migrations.
    pub async fn open(database_url: &str, max_connections: u32) -> Result<Self, StoreError> {
        let pool = connect(database_url, max_connections)
            .await
            .map_err(|e| StoreError::Connection(Box::new(e)))?;
        run_migrations(&pool)
            .await
            .map_err(|e| StoreError::Migration(Box::new(e)))?;
        Ok(Self::from_pool(pool))
    }

    /// Wrap an already-connected, already-migrated pool (bins and test harnesses
    /// that build the pool themselves).
    pub fn from_pool(pool: PgPool) -> Self {
        let accounts = Accounts::new(pool.clone());
        Self { pool, accounts }
    }

    /// Raw pool access, for the stress bins that issue their own SQL. Not part
    /// of `StorageEngine`: no engine-agnostic caller may assume a Postgres pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait::async_trait]
impl StorageEngine for PgStorage {
    async fn create_account(&self, external_ref: Option<&str>) -> Result<AccountRow, StoreError> {
        self.accounts.create(external_ref).await
    }

    async fn list_accounts(&self) -> Result<Vec<AccountRow>, StoreError> {
        self.accounts.list().await
    }

    async fn delete_account(&self, uuid: uuid::Uuid) -> Result<bool, StoreError> {
        self.accounts.delete(uuid).await
    }

    fn device_backend(&self, device_id: i32) -> std::sync::Arc<dyn Backend> {
        std::sync::Arc::new(PgBackend::new(self.pool.clone(), device_id))
    }

    async fn ping_storage(&self) -> bool {
        // A trivial round-trip: proves the pool can hand out a live connection,
        // which is exactly what readiness means here.
        sqlx::query("SELECT 1").execute(&self.pool).await.is_ok()
    }
}
