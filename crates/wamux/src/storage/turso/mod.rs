//! The Turso engine family (#106): the native async `turso` crate, a second
//! family behind `StorageEngine` next to `sql`. Not a `SqlPool` arm: turso is
//! not sqlx, its API and its connection model are its own.
//!
//! The statement text is the `sql` family's (`storage::statements`), rewritten
//! from `$N` to `?N` before it is prepared: turso binds `$N` by order of first
//! appearance, not by number, which silently writes values into the wrong
//! columns (pinned in `placeholders_tests`).
//!
//! One connection behind a `tokio::sync::Mutex`, the equivalent of the SQLite
//! engine's one-connection pool. A cloned `turso::Connection` shares its
//! transaction state, so concurrent use is a misuse error, and two connections
//! on one file contend with `Busy` (see `TursoConn`).
//!
//! Paths are written `::turso::...`: this module is also named `turso`.

mod accounts;
mod app_sync_sql;
mod app_sync_store;
mod bincode_upgrade;
mod connection;
mod convert;
mod device_store;
mod dsn;
mod exec;
mod maintenance_sql;
mod migrations;
mod msg_secret_store;
mod placeholders;
mod protocol_batch_sql;
mod protocol_decode;
mod protocol_store;
mod row_values;
mod signal_sql;
mod signal_store;
mod store_encryption_sql;
mod tc_token_sql;
mod transaction;
mod turso_error;

#[cfg(test)]
mod dsn_tests;
#[cfg(test)]
mod placeholders_tests;

use std::path::Path;
use std::sync::Arc;

use uuid::Uuid;
use wacore::store::error::{Result as StoreResult, StoreError};
use wacore::store::traits::Backend;

use crate::storage::blob_cipher::BlobCipher;
use crate::storage::engine::{AccountRow, StorageEngine};
use crate::storage::statements::PING;
use crate::storage::store_key::StoreKey;

pub use connection::TursoConn;
pub(crate) use dsn::turso_path;
pub use migrations::TursoMigrationError;
pub(crate) use placeholders::turso_placeholders;
pub(crate) use transaction::TursoTx;

/// The Turso engine: one connection, the `accounts` table, one `TursoBackend`
/// per account.
#[derive(Clone)]
pub struct TursoStore {
    conn: TursoConn,
    cipher: BlobCipher,
}

impl TursoStore {
    /// Open (creating if absent) the file a `turso://<path>` DSN names and
    /// apply pending migrations.
    pub async fn open(database_url: &str) -> StoreResult<Self> {
        Self::open_keyed(database_url, None).await
    }

    /// `open` with the store key (#164).
    pub async fn open_keyed(database_url: &str, key: Option<&StoreKey>) -> StoreResult<Self> {
        Self::open_path_keyed(Path::new(turso_path(database_url)?), key).await
    }

    /// Open (creating if absent) the file at `path` and apply pending
    /// migrations: `migrations_sqlite/`, recorded in `_sqlx_migrations` the way
    /// sqlx records them, then the bincode upgrade (#31), the same neutral
    /// conversion every engine runs. A failure anywhere drops the connection
    /// before returning, which frees the file.
    pub async fn open_path(path: &Path) -> StoreResult<Self> {
        Self::open_path_keyed(path, None).await
    }

    /// `open_path` with the store key (#164): once migrated, the store's
    /// encryption state is settled against the key before any backend exists.
    async fn open_path_keyed(path: &Path, key: Option<&StoreKey>) -> StoreResult<Self> {
        crate::storage::file_mode::secure_store_file(path)?;
        let conn = TursoConn::open(path).await?;
        migrations::apply_pending(&conn)
            .await
            .map_err(|e| StoreError::Migration(Box::new(e)))?;
        bincode_upgrade::upgrade_bincode_blobs(&conn)
            .await
            .map_err(|e| StoreError::Migration(Box::new(e)))?;
        let cipher = store_encryption_sql::open_cipher(&conn, key).await?;
        Ok(Self { conn, cipher })
    }

    /// `wamux store decrypt` on a turso store (#165): migrate, decrypt every
    /// account still sealed, and release the file before returning.
    pub async fn decrypt(database_url: &str, key: &StoreKey) -> StoreResult<usize> {
        let path = Path::new(turso_path(database_url)?);
        crate::storage::file_mode::secure_store_file(path)?;
        let conn = TursoConn::open(path).await?;
        let decrypted = Self::decrypt_open(&conn, key).await;
        conn.close().await?;
        decrypted
    }

    async fn decrypt_open(conn: &TursoConn, key: &StoreKey) -> StoreResult<usize> {
        migrations::apply_pending(conn)
            .await
            .map_err(|e| StoreError::Migration(Box::new(e)))?;
        bincode_upgrade::upgrade_bincode_blobs(conn)
            .await
            .map_err(|e| StoreError::Migration(Box::new(e)))?;
        store_encryption_sql::decrypt_store(conn, key).await
    }

    /// Close the connection and release the file, its lock included, before
    /// returning. Another opener of the same file in this process (a sqlx
    /// pool, a second `TursoStore`) must wait for this: two holders in one
    /// process share one POSIX lock, and either one closing drops it. Fails,
    /// leaving the file open, while a backend or a clone is still alive.
    pub async fn close(self) -> StoreResult<()> {
        self.conn.close().await
    }

    /// Raw connection access for callers that issue their own SQL (tests).
    /// Not part of `StorageEngine`.
    pub fn connection(&self) -> &TursoConn {
        &self.conn
    }
}

#[async_trait::async_trait]
impl StorageEngine for TursoStore {
    async fn create_account(&self, external_ref: Option<&str>) -> StoreResult<AccountRow> {
        accounts::create_account(&self.conn, external_ref).await
    }

    async fn list_accounts(&self) -> StoreResult<Vec<AccountRow>> {
        accounts::list_accounts(&self.conn).await
    }

    async fn delete_account(&self, uuid: Uuid) -> StoreResult<bool> {
        accounts::delete_account(&self.conn, uuid).await
    }

    fn device_backend(&self, device_id: i32) -> Arc<dyn Backend> {
        Arc::new(TursoBackend {
            conn: self.conn.clone(),
            device_id,
            cipher: self.cipher.clone(),
        })
    }

    async fn ping_storage(&self) -> bool {
        // A trivial round-trip: proves the connection answers, which is exactly
        // what readiness means here.
        self.conn.fetch_all(PING, Vec::new()).await.is_ok()
    }
}

/// wacore's storage backend bound to one account's `device_id`.
#[derive(Clone)]
pub struct TursoBackend {
    pub(crate) conn: TursoConn,
    pub(crate) device_id: i32,
    pub(crate) cipher: BlobCipher,
}

impl TursoBackend {
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
