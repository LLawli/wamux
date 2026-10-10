//! A transaction on the one turso connection (#106): a job of the group commit
//! (#172), which is a SAVEPOINT inside the batch's transaction. The crate's own
//! `Transaction` needs `&mut Connection` and, dropped, only marks a rollback on
//! the clone that opened it, which is no use on a shared connection.
//!
//! The job holds the connection from `begin` to `commit`, so nothing else runs
//! in between. Inside a transaction a constraint violation reverts only its
//! statement and the job carries on (measured on 0.8.1), so a failed job must end
//! in `rollback`. Early returns do that through the group commit: a job dropped
//! without `commit` or `rollback` is rolled back to its savepoint by the next
//! user of the connection. `commit` returns once the COMMIT that carries the job
//! ran, which may be one shared with other writers.

use ::turso::{Connection, Row};
use wacore::store::error::Result as StoreResult;

use super::TursoConn;
use super::exec::{self, Binds};
use crate::storage::group_commit::JobGuard;

pub(crate) struct TursoTx<'c> {
    job: JobGuard<'c, Connection>,
}

impl<'c> TursoTx<'c> {
    pub(crate) async fn begin(conn: &'c TursoConn) -> StoreResult<TursoTx<'c>> {
        Ok(Self {
            job: conn.begin_job().await?,
        })
    }

    pub(crate) async fn execute(&self, sql: &str, binds: Binds) -> StoreResult<u64> {
        exec::execute(&self.job, sql, binds).await
    }

    pub(crate) async fn fetch_all(&self, sql: &str, binds: Binds) -> StoreResult<Vec<Row>> {
        exec::fetch_all(&self.job, sql, binds).await
    }

    pub(crate) async fn fetch_optional(&self, sql: &str, binds: Binds) -> StoreResult<Option<Row>> {
        exec::fetch_optional(&self.job, sql, binds).await
    }

    /// End the job, discarding its writes.
    pub(crate) async fn rollback(self) {
        self.job.fail().await;
    }

    pub(crate) async fn commit(self) -> StoreResult<()> {
        self.job.finish().await
    }
}
