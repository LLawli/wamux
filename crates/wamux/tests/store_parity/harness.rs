//! Engine setup, two-account fixtures, and the raw reads below the trait.

use std::sync::Arc;

use async_trait::async_trait;
use wacore::store::traits::Backend;
use wamux::storage::postgres::PgStorage;
use wamux::storage::sqlite::SqliteStorage;
use wamux::storage::{AccountRow, StorageEngine};

use crate::common;

/// Reads that bypass the traits on purpose: some contracts are only visible in
/// the stored row (the `uploaded` flag has no getter), and byte parity is a
/// claim about the column, not about what each engine reads back to itself.
///
/// Table and column names come from test constants, never from input, so
/// formatting them into the SQL is safe here.
#[async_trait]
pub trait RawProbe: Send + Sync {
    /// Every value of a BLOB/BYTEA `table.column` for one account, sorted.
    async fn column_bytes(&self, table: &str, column: &str, device_id: i32) -> Vec<Vec<u8>>;

    /// Every value of a TEXT `table.column` for one account, sorted.
    async fn column_text(&self, table: &str, column: &str, device_id: i32) -> Vec<String>;

    /// `prekeys.uploaded` for one row; `None` when the row does not exist.
    async fn prekey_uploaded(&self, device_id: i32, id: u32) -> Option<bool>;
}

#[async_trait]
impl RawProbe for PgStorage {
    async fn column_bytes(&self, table: &str, column: &str, device_id: i32) -> Vec<Vec<u8>> {
        let sql = format!("SELECT {column} FROM {table} WHERE device_id = $1");
        let mut rows: Vec<Vec<u8>> = sqlx::query_scalar(&sql)
            .bind(device_id)
            .fetch_all(self.pool())
            .await
            .unwrap_or_else(|e| panic!("read {table}.{column} on postgres: {e}"));
        rows.sort();
        rows
    }

    async fn column_text(&self, table: &str, column: &str, device_id: i32) -> Vec<String> {
        let sql = format!("SELECT {column} FROM {table} WHERE device_id = $1");
        let mut rows: Vec<String> = sqlx::query_scalar(&sql)
            .bind(device_id)
            .fetch_all(self.pool())
            .await
            .unwrap_or_else(|e| panic!("read {table}.{column} on postgres: {e}"));
        rows.sort();
        rows
    }

    async fn prekey_uploaded(&self, device_id: i32, id: u32) -> Option<bool> {
        sqlx::query_scalar("SELECT uploaded FROM prekeys WHERE device_id = $1 AND id = $2")
            .bind(device_id)
            .bind(id as i32)
            .fetch_optional(self.pool())
            .await
            .expect("read prekeys.uploaded on postgres")
    }
}

#[async_trait]
impl RawProbe for SqliteStorage {
    async fn column_bytes(&self, table: &str, column: &str, device_id: i32) -> Vec<Vec<u8>> {
        let sql = format!("SELECT {column} FROM {table} WHERE device_id = ?");
        let mut rows: Vec<Vec<u8>> = sqlx::query_scalar(&sql)
            .bind(device_id)
            .fetch_all(self.pool())
            .await
            .unwrap_or_else(|e| panic!("read {table}.{column} on sqlite: {e}"));
        rows.sort();
        rows
    }

    async fn column_text(&self, table: &str, column: &str, device_id: i32) -> Vec<String> {
        let sql = format!("SELECT {column} FROM {table} WHERE device_id = ?");
        let mut rows: Vec<String> = sqlx::query_scalar(&sql)
            .bind(device_id)
            .fetch_all(self.pool())
            .await
            .unwrap_or_else(|e| panic!("read {table}.{column} on sqlite: {e}"));
        rows.sort();
        rows
    }

    // SQLite keeps the flag as INTEGER 0/1; sqlx decodes it as bool, so the
    // two engines answer in the same type.
    async fn prekey_uploaded(&self, device_id: i32, id: u32) -> Option<bool> {
        sqlx::query_scalar("SELECT uploaded FROM prekeys WHERE device_id = ? AND id = ?")
            .bind(device_id)
            .bind(id as i32)
            .fetch_optional(self.pool())
            .await
            .expect("read prekeys.uploaded on sqlite")
    }
}

/// One engine under test. The SQLite temp dir lives as long as the harness.
pub struct Harness {
    pub storage: Arc<dyn StorageEngine>,
    pub raw: Arc<dyn RawProbe>,
    _dir: Option<tempfile::TempDir>,
}

/// The dockerized Postgres (`DATABASE_URL`). Tests share the database, so
/// every fixture scopes itself to its own accounts.
pub async fn postgres() -> Harness {
    let storage = common::pg_engine(5).await;
    Harness {
        storage: storage.clone(),
        raw: storage,
        _dir: None,
    }
}

/// A fresh SQLite file per test.
pub async fn sqlite() -> Harness {
    let (storage, dir) = common::sqlite_engine().await;
    Harness {
        storage: storage.clone(),
        raw: storage,
        _dir: Some(dir),
    }
}

/// Two accounts on one engine. Every body writes through `ba` and checks that
/// `bb` never sees it: the `device_id` scoping is half of each contract.
pub struct TwoAccounts {
    pub a: AccountRow,
    pub b: AccountRow,
    pub ba: Arc<dyn Backend>,
    pub bb: Arc<dyn Backend>,
}

impl Harness {
    /// `tag` must be unique per test: it prefixes the accounts' `external_ref`
    /// and is swept first, so a run that aborted before its teardown cannot
    /// leave rows behind for the next one. A shared prefix would let one test
    /// sweep another's accounts while it runs, and that includes one tag being
    /// a prefix of another (`prekeys` of `prekeys-uploaded`): hence the `/`,
    /// which no tag contains.
    pub async fn two_accounts(&self, tag: &str) -> TwoAccounts {
        assert!(!tag.contains('/'), "tag {tag:?} must not contain '/'");
        let prefix = format!("parity-{tag}/");
        common::sweep_orphans(&self.storage, &prefix).await;
        let a = self.create(&format!("{prefix}a")).await;
        let b = self.create(&format!("{prefix}b")).await;
        assert_ne!(a.device_id, b.device_id, "device_ids must differ");
        TwoAccounts {
            ba: self.storage.device_backend(a.device_id),
            bb: self.storage.device_backend(b.device_id),
            a,
            b,
        }
    }

    async fn create(&self, external_ref: &str) -> AccountRow {
        self.storage
            .create_account(Some(external_ref))
            .await
            .unwrap_or_else(|e| panic!("create account {external_ref}: {e}"))
    }

    /// Delete both accounts; the cascade takes every scoped row with them.
    pub async fn drop_accounts(&self, pair: TwoAccounts) {
        for row in [pair.a, pair.b] {
            assert!(
                self.storage.delete_account(row.uuid).await.unwrap(),
                "account {} must exist at teardown",
                row.uuid
            );
        }
    }
}

/// Unix seconds now, for the stores that stamp rows with the wall clock
/// (`sent_messages.created_at`). Tests use it only with an hour of margin.
pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after 1970")
        .as_secs() as i64
}
