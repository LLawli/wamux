//! A transaction on whichever driver the pool holds. Statements go through
//! `on_sql_tx!`, so a multi-statement operation is a plain loop written once.
//!
//! On SQLite a transaction is a job of the group commit (#173): a SAVEPOINT
//! inside the batch's transaction, holding the writer from `begin` to `commit`.
//! A `SqlTx` dropped without `commit` is an abandoned job, rolled back to its
//! savepoint by the next user of the writer (what the turso engine does too),
//! with a warning. A caller that decides to stop on purpose calls `rollback`.

use sqlx::{Postgres, SqliteConnection, Transaction};
use wacore::store::error::{Result, StoreError};

use super::SqlPool;
use crate::storage::group_commit::JobGuard;
use crate::storage::sqlx_error::db;

pub(crate) enum SqlTx<'c> {
    Pg(Transaction<'c, Postgres>),
    Sqlite(JobGuard<'c, SqliteConnection>),
}

impl<'c> SqlTx<'c> {
    /// Open a transaction. `sqlx::Error` is returned as is so a caller with its
    /// own error type (the bincode upgrade) can add its own context; a SQLite
    /// job that cannot start (its BEGIN failed) is wrapped as `sqlx::Error::Protocol`
    /// carrying the store error's text.
    pub(crate) async fn begin_raw(
        pool: &'c SqlPool,
    ) -> std::result::Result<SqlTx<'c>, sqlx::Error> {
        match pool {
            SqlPool::Pg(pool) => Ok(SqlTx::Pg(pool.begin().await?)),
            SqlPool::Sqlite(lite) => lite
                .begin_job()
                .await
                .map(SqlTx::Sqlite)
                .map_err(store_error_as_sqlx),
        }
    }

    pub(crate) async fn begin(pool: &'c SqlPool) -> Result<SqlTx<'c>> {
        match pool {
            SqlPool::Pg(pool) => Ok(SqlTx::Pg(pool.begin().await.map_err(db)?)),
            SqlPool::Sqlite(lite) => lite.begin_job().await.map(SqlTx::Sqlite),
        }
    }

    pub(crate) async fn commit_raw(self) -> std::result::Result<(), sqlx::Error> {
        match self {
            SqlTx::Pg(tx) => tx.commit().await,
            SqlTx::Sqlite(job) => job.finish().await.map_err(store_error_as_sqlx),
        }
    }

    /// End the transaction on purpose, discarding its writes. On SQLite this
    /// rolls the job back to its savepoint now, so no abandoned job is left to
    /// warn about.
    pub(crate) async fn rollback(self) -> Result<()> {
        match self {
            SqlTx::Pg(tx) => tx.rollback().await.map_err(db),
            SqlTx::Sqlite(job) => {
                job.fail().await;
                Ok(())
            }
        }
    }

    pub(crate) async fn commit(self) -> Result<()> {
        match self {
            SqlTx::Pg(tx) => tx.commit().await.map_err(db),
            SqlTx::Sqlite(job) => job.finish().await,
        }
    }
}

/// The group commit speaks `StoreError`; `begin_raw` and `commit_raw` speak
/// `sqlx::Error` for the bincode upgrade, which wraps it with its own context.
/// Keeping the message keeps the cause visible in that error.
fn store_error_as_sqlx(error: StoreError) -> sqlx::Error {
    sqlx::Error::Protocol(error.to_string())
}
