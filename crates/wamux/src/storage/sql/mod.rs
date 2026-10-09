//! The SQL engine family (#65): Postgres and SQLite through sqlx, every
//! statement written once.
//!
//! `SqlBackend` implements wacore's store traits (`SignalStore`, `AppSyncStore`,
//! `ProtocolStore`, `DeviceStore`, plus `MsgSecretStore`) on a `SqlPool`; since
//! `Backend` is a blanket impl over them, that makes it a `Backend`. Statements
//! are `$N` strings that run on both drivers; `macros` expands each one per
//! driver. The text lives in `storage::statements` (#106), shared with the
//! turso family, so no SQL is spelled here except the forms with no common
//! shape: `prekeys_sql` (Postgres array bind) and the `blob_format` row lock in
//! `bincode_upgrade`. Code per driver, with no SQL of its own, stays in
//! `accounts` (UUID vs TEXT binds) and `maintenance_sql` (SQLite upkeep vs
//! Postgres autovacuum).
//!
//! Multi-tenancy: one shared pool, one `SqlBackend` per account, each carrying
//! the integer `device_id` that scopes every row. The engine that is not sqlx
//! (#106) is a sibling family behind `StorageEngine`, not a variant here.

#[macro_use]
mod macros;

mod accounts;
mod app_sync_sql;
mod app_sync_store;
mod bincode_upgrade;
mod connect;
mod device_store;
mod maintenance_sql;
mod msg_secret_store;
mod prekeys_sql;
mod protocol_batch_sql;
mod protocol_store;
mod signal_sql;
mod signal_store;
mod store_encryption_sql;
mod tc_token_sql;
mod transaction;

use std::str::FromStr;
use std::sync::Arc;

use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{PgPool, SqlitePool};
use uuid::Uuid;
use wacore::store::error::{Result as StoreResult, StoreError};
use wacore::store::traits::Backend;

use crate::storage::blob_cipher::BlobCipher;
use crate::storage::engine::{AccountRow, StorageEngine};
use crate::storage::file_mode;
use crate::storage::statements::PING;
use crate::storage::store_key::StoreKey;

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
    cipher: BlobCipher,
}

impl SqlStore {
    /// Connect to Postgres and apply pending migrations. No store key: a
    /// plaintext store, or an encrypted one is refused (#164).
    pub async fn open_postgres(database_url: &str, max_connections: u32) -> StoreResult<Self> {
        Self::open_postgres_keyed(database_url, max_connections, None).await
    }

    /// `open_postgres` with the store key (#164).
    pub async fn open_postgres_keyed(
        database_url: &str,
        max_connections: u32,
        key: Option<&StoreKey>,
    ) -> StoreResult<Self> {
        let pool = connect_postgres(database_url, max_connections)
            .await
            .map_err(|e| StoreError::Connection(Box::new(e)))?;
        Self::migrated(SqlPool::Pg(pool), key).await
    }

    /// Open (creating if absent) the SQLite file and apply pending migrations.
    pub async fn open_sqlite(database_url: &str) -> StoreResult<Self> {
        Self::open_sqlite_keyed(database_url, None).await
    }

    /// `open_sqlite` with the store key (#164).
    pub async fn open_sqlite_keyed(
        database_url: &str,
        key: Option<&StoreKey>,
    ) -> StoreResult<Self> {
        secure_sqlite_file(database_url)?;
        let pool = connect_sqlite(database_url)
            .await
            .map_err(|e| StoreError::Connection(Box::new(e)))?;
        Self::migrated(SqlPool::Sqlite(pool), key).await
    }

    /// Migrate, then settle the store's encryption state against `key` before any
    /// backend exists (#164): a wrong or missing key stops here.
    async fn migrated(pool: SqlPool, key: Option<&StoreKey>) -> StoreResult<Self> {
        run_migrations(&pool).await?;
        let cipher = store_encryption_sql::open_cipher(&pool, key).await?;
        Ok(Self { pool, cipher })
    }

    /// Wrap an already-connected, already-migrated pool (bins and test
    /// harnesses that build the pool themselves). No key: the blobs are read and
    /// written as they are, so it must not be pointed at an encrypted store.
    pub fn from_pool(pool: SqlPool) -> Self {
        Self {
            pool,
            cipher: BlobCipher::passthrough(),
        }
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
        Arc::new(SqlBackend::with_cipher(
            self.pool.clone(),
            device_id,
            self.cipher.clone(),
        ))
    }

    async fn ping_storage(&self) -> bool {
        // A trivial round-trip: proves the pool can hand out a live connection,
        // which is exactly what readiness means here.
        on_sql_pool!(&self.pool, |conn| {
            sqlx::query(PING).execute(conn).await.is_ok()
        })
    }
}

/// wacore's storage backend bound to one account's `device_id`.
#[derive(Clone)]
pub struct SqlBackend {
    pub(crate) pool: SqlPool,
    pub(crate) device_id: i32,
    pub(crate) cipher: BlobCipher,
}

impl SqlBackend {
    /// A backend with no key: bytes in and out as they are (the stress bins).
    pub fn new(pool: SqlPool, device_id: i32) -> Self {
        Self::with_cipher(pool, device_id, BlobCipher::passthrough())
    }

    pub fn with_cipher(pool: SqlPool, device_id: i32, cipher: BlobCipher) -> Self {
        Self {
            pool,
            device_id,
            cipher,
        }
    }

    pub fn device_id(&self) -> i32 {
        self.device_id
    }

    /// Seal one value of this account's sealed column (#164).
    pub(crate) fn seal_at(
        &self,
        table: &'static str,
        column: &'static str,
        row: &[u8],
        plain: &[u8],
    ) -> StoreResult<Vec<u8>> {
        self.cipher
            .seal_at(self.device_id, table, column, row, plain)
    }

    /// Open one value read from this account's sealed column (#164).
    pub(crate) fn open_at(
        &self,
        table: &'static str,
        column: &'static str,
        row: &[u8],
        stored: &[u8],
    ) -> StoreResult<Vec<u8>> {
        self.cipher
            .open_at(self.device_id, table, column, row, stored)
    }
}

/// Create the SQLite file 0600 before sqlx would create it with the umask's
/// mode (#75). In-memory databases have no file.
fn secure_sqlite_file(database_url: &str) -> StoreResult<()> {
    let options = SqliteConnectOptions::from_str(database_url)
        .map_err(|e| StoreError::Connection(Box::new(e)))?;
    if database_url.contains(":memory:") || database_url.contains("mode=memory") {
        return Ok(());
    }
    file_mode::secure_store_file(options.get_filename())
}
