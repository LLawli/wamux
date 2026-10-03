//! A transaction on whichever driver the pool holds. Statements go through
//! `on_sql_tx!`, so a multi-statement operation is a plain loop written once.

use sqlx::{Postgres, Sqlite, Transaction};
use wacore::store::error::Result;

use super::SqlPool;
use crate::storage::sqlx_error::db;

pub(crate) enum SqlTx<'c> {
    Pg(Transaction<'c, Postgres>),
    Sqlite(Transaction<'c, Sqlite>),
}

impl SqlTx<'_> {
    /// Open a transaction. `sqlx::Error` is returned as is so a caller with its
    /// own error type (the bincode upgrade) can add its own context.
    pub(crate) async fn begin_raw(
        pool: &SqlPool,
    ) -> std::result::Result<SqlTx<'static>, sqlx::Error> {
        match pool {
            SqlPool::Pg(pool) => Ok(SqlTx::Pg(pool.begin().await?)),
            SqlPool::Sqlite(pool) => Ok(SqlTx::Sqlite(pool.begin().await?)),
        }
    }

    pub(crate) async fn begin(pool: &SqlPool) -> Result<SqlTx<'static>> {
        Self::begin_raw(pool).await.map_err(db)
    }

    pub(crate) async fn commit_raw(self) -> std::result::Result<(), sqlx::Error> {
        match self {
            SqlTx::Pg(tx) => tx.commit().await,
            SqlTx::Sqlite(tx) => tx.commit().await,
        }
    }

    pub(crate) async fn commit(self) -> Result<()> {
        self.commit_raw().await.map_err(db)
    }
}
