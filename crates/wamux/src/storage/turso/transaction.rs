//! A transaction on the one turso connection (#106): plain `BEGIN` / `COMMIT` /
//! `ROLLBACK` on the locked connection. The crate's own `Transaction` needs
//! `&mut Connection` and, dropped, only marks a rollback on the clone that
//! opened it, which is no use on a shared connection.
//!
//! The guard is held from BEGIN to COMMIT, so nothing else runs in between. Inside
//! BEGIN a constraint violation reverts only its statement and the transaction
//! carries on (measured on 0.8.1), so a failed batch must end in ROLLBACK. Early
//! returns do that through `TursoConn::lock`, which rolls back an abandoned
//! transaction before handing the connection on; `commit` rolls back itself when
//! COMMIT fails.

use ::turso::{Connection, Row};
use tokio::sync::MutexGuard;
use wacore::store::error::Result as StoreResult;

use super::TursoConn;
use super::exec::{self, Binds};
use super::turso_error::db;

pub(crate) struct TursoTx<'c> {
    guard: MutexGuard<'c, Connection>,
}

impl<'c> TursoTx<'c> {
    pub(crate) async fn begin(conn: &'c TursoConn) -> StoreResult<TursoTx<'c>> {
        let guard = conn.lock().await;
        guard.execute("BEGIN", ()).await.map_err(db)?;
        Ok(Self { guard })
    }

    pub(crate) async fn execute(&self, sql: &str, binds: Binds) -> StoreResult<u64> {
        exec::execute(&self.guard, sql, binds).await
    }

    pub(crate) async fn fetch_all(&self, sql: &str, binds: Binds) -> StoreResult<Vec<Row>> {
        exec::fetch_all(&self.guard, sql, binds).await
    }

    pub(crate) async fn fetch_optional(&self, sql: &str, binds: Binds) -> StoreResult<Option<Row>> {
        exec::fetch_optional(&self.guard, sql, binds).await
    }

    /// End the transaction, discarding it. Best effort: the connection's next
    /// user rolls back an open transaction anyway (`TursoConn::lock`).
    pub(crate) async fn rollback(self) {
        let _ = self.guard.execute("ROLLBACK", ()).await;
    }

    pub(crate) async fn commit(self) -> StoreResult<()> {
        let committed = self.guard.execute("COMMIT", ()).await;
        if committed.is_err() {
            // The transaction is still open after a failed COMMIT.
            let _ = self.guard.execute("ROLLBACK", ()).await;
        }
        committed.map(drop).map_err(db)
    }
}
