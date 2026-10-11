//! The two handles a SQLite store runs on (#173): one writer inside the group
//! commit, and a read-only pool on the same file.
//!
//! The writer is a single `SqliteConnection` behind `GroupCommit`, the same
//! shape as the turso engine (`storage/turso/connection.rs`): every write is a
//! job of a batch, and the accounts that queue together share one COMMIT, which
//! is what makes `synchronous = FULL` affordable (#169). The readers are other
//! connections, opened `read_only`, so a read never waits for the batch and, in
//! WAL, never sees a write that has no COMMIT yet. They cannot write at all,
//! which is what keeps the "two writing connections" SQLITE_BUSY trap in
//! `connect_sqlite` from applying to them.

use std::sync::Arc;

use sqlx::sqlite::SqliteConnection;
use sqlx::{Connection, SqlitePool};
use wacore::store::error::{Result as StoreResult, StoreError};

use crate::storage::group_commit::{
    BatchConn, CommitStats, ConnGuard, ControlStatement, GroupCommit, JobGuard,
};
use crate::storage::sqlx_error::db;

/// How many connections the read pool may open. Reads are short point lookups
/// (one session, one prekey), so 4 lets the accounts read side by side without
/// holding many file descriptors; a larger pool would only add fds, since the
/// reads never wait on a writer (#173).
pub(super) const SQLITE_READERS: u32 = 4;

/// The control statements of the batch, run as plain statements on the writer.
#[async_trait::async_trait]
impl BatchConn for SqliteConnection {
    async fn control(&mut self, statement: ControlStatement) -> StoreResult<()> {
        sqlx::query(statement.sql())
            .execute(&mut *self)
            .await
            .map(drop)
            .map_err(db)
    }
}

/// Field order is drop order: the writer goes first, then the readers.
struct SqliteInner {
    group: GroupCommit<SqliteConnection>,
    readers: SqlitePool,
}

/// The writer and the readers of one SQLite file, shared by every account.
#[derive(Clone)]
pub struct SqliteConn(Arc<SqliteInner>);

impl SqliteConn {
    /// Wrap a writer that already opened the file and the readers opened after it.
    pub(super) fn new(writer: SqliteConnection, readers: SqlitePool, batch_cap: usize) -> Self {
        let group = GroupCommit::new(writer, batch_cap);
        Self(Arc::new(SqliteInner { group, readers }))
    }

    /// The read-only pool every read runs on.
    pub fn read_pool(&self) -> &SqlitePool {
        &self.0.readers
    }

    /// The writer, outside any job: for tools and tests that write raw SQL. An
    /// abandoned job is rolled back first, and an open batch with no writer
    /// pending is committed.
    pub async fn lock(&self) -> ConnGuard<'_, SqliteConnection> {
        self.0.group.lock().await
    }

    /// The writer outside any transaction, for what cannot run inside one: the
    /// WAL checkpoint ("database table is locked" inside a transaction, #173),
    /// VACUUM and the migrations.
    pub(crate) async fn lock_autocommit(&self) -> StoreResult<ConnGuard<'_, SqliteConnection>> {
        self.0.group.lock_autocommit().await
    }

    /// Start a job in the batch: the entry of every write.
    pub(crate) async fn begin_job(&self) -> StoreResult<JobGuard<'_, SqliteConnection>> {
        self.0.group.begin_job().await
    }

    /// How many COMMITs carried how many jobs, since the store opened.
    pub fn commit_stats(&self) -> CommitStats {
        self.0.group.stats()
    }

    /// Commit what is pending and close the readers and the writer. Only
    /// possible when no other handle (a backend, a clone) is alive: they share
    /// the one writer. The writer closes last, so it is the connection that
    /// leaves the WAL clean.
    pub async fn close(self) -> StoreResult<()> {
        // Jobs whose callers went away may still wait for their COMMIT.
        drop(self.lock_autocommit().await?);
        let others = Arc::strong_count(&self.0) - 1;
        let inner = Arc::try_unwrap(self.0).map_err(|_| {
            StoreError::Connection(
                format!("sqlite store still in use by {others} other handle(s); drop them first")
                    .into(),
            )
        })?;
        let SqliteInner { group, readers } = inner;
        readers.close().await;
        group.into_inner().close().await.map_err(db)
    }
}

/// End a SQLite job by how its statement went: commit on success, roll back to
/// its savepoint on failure. The one place that rule is written.
pub(crate) async fn end_job<T>(
    job: JobGuard<'_, SqliteConnection>,
    ran: StoreResult<T>,
) -> StoreResult<T> {
    match ran {
        Ok(value) => job.finish().await.map(|()| value),
        Err(error) => {
            job.fail().await;
            Err(error)
        }
    }
}
