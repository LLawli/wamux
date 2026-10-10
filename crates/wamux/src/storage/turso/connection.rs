//! The one connection a `TursoStore` runs on (#106), and how it is opened.
//!
//! One `turso::Connection` behind the group commit's `tokio::sync::Mutex`
//! (#172), the equivalent of the SQLite engine's `max_connections(1)`. Why
//! one, measured on turso 0.8.1: a
//! cloned `Connection` shares its transaction state, so two tasks using it at
//! once get `Misuse("concurrent use forbidden")` and a commit that was not
//! atomic; and two connections on one file contend with `Busy`. The mutex is
//! `tokio`, not `std`: its guard is held across every `.await` of a statement,
//! and of a whole transaction from BEGIN to COMMIT, because a second user in
//! between would join that transaction.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ::turso::{Builder, Connection, Database};
use wacore::store::error::{Result as StoreResult, StoreError};

use super::exec;
use super::turso_error::db;
use crate::storage::group_commit::{
    BatchConn, CommitStats, ConnGuard, ControlStatement, GroupCommit, JobGuard,
};

/// Same patience as the SQLite engine's `busy_timeout`.
const BUSY_TIMEOUT: Duration = Duration::from_secs(30);

/// The connection and the `Database` it came from. Field order is drop order:
/// the connection goes first, then the handle that closes the file.
struct ConnInner {
    group: GroupCommit<Connection>,
    db: Database,
}

/// The control statements of the batch, run as plain statements (#172).
#[async_trait::async_trait]
impl BatchConn for Connection {
    async fn control(&mut self, statement: ControlStatement) -> StoreResult<()> {
        self.execute(statement.sql(), ())
            .await
            .map(drop)
            .map_err(db)
    }
}

/// The one connection a `TursoStore` runs on, shared by every account.
#[derive(Clone)]
pub struct TursoConn(Arc<ConnInner>);

impl TursoConn {
    /// Open (creating if absent) the database file with the engine's pragmas.
    pub(super) async fn open(path: &Path, batch_cap: usize) -> StoreResult<Self> {
        let path = path.to_str().ok_or_else(|| {
            StoreError::InvalidConfig(format!("turso path is not valid UTF-8: {path:?}"))
        })?;
        // `build` also converts the file header to WAL (the turso default).
        // #165: turso 0.8.1 refuses VACUUM unless it is switched on. The conversion
        // needs it to drop the plaintext the sealing left in the file (tested: the
        // file stays whole for sqlite3 and sqlx, and the old bytes are gone).
        let database = Builder::new_local(path)
            .experimental_vacuum(true)
            .build()
            .await
            .map_err(|e| StoreError::Connection(Box::new(e)))?;
        let conn = database.connect().map_err(db)?;
        apply_pragmas(&conn).await?;
        let inner = ConnInner {
            group: GroupCommit::new(conn, batch_cap),
            db: database,
        };
        Ok(Self(Arc::new(inner)))
    }

    /// The connection for a read (#172): an abandoned job is rolled back, and a
    /// batch in progress is either committed (no writer pending) or seen as is.
    pub async fn lock(&self) -> ConnGuard<'_, Connection> {
        self.0.group.lock().await
    }

    /// The connection outside any transaction, for what cannot run inside one:
    /// the checkpoint, VACUUM and the migrations.
    pub(super) async fn lock_autocommit(&self) -> StoreResult<ConnGuard<'_, Connection>> {
        self.0.group.lock_autocommit().await
    }

    /// Start a job in the batch: the entry of every write.
    pub(super) async fn begin_job(&self) -> StoreResult<JobGuard<'_, Connection>> {
        self.0.group.begin_job().await
    }

    pub(super) fn commit_stats(&self) -> CommitStats {
        self.0.group.stats()
    }

    /// Release the file before returning. Only possible when no other handle
    /// (a backend, a clone) is alive: they share the one connection.
    pub(super) async fn close(self) -> StoreResult<()> {
        // Jobs whose callers went away may still wait for their COMMIT.
        drop(self.lock_autocommit().await?);
        let others = Arc::strong_count(&self.0) - 1;
        let inner = Arc::try_unwrap(self.0).map_err(|_| {
            StoreError::Connection(
                format!("turso store still in use by {others} other handle(s); drop them first")
                    .into(),
            )
        })?;
        let ConnInner { group, db } = inner;
        drop(group.into_inner());
        drop(db);
        Ok(())
    }
}

/// `foreign_keys` is per connection and defaults OFF here too: without it,
/// deleting an account orphans every Signal row it owns (#106 probe on turso
/// 0.8.1, same trap as SQLite). `synchronous = FULL`: turso documents only OFF
/// and FULL, and FULL is the one that does not lose a commit on power loss.
async fn apply_pragmas(conn: &Connection) -> StoreResult<()> {
    conn.busy_timeout(BUSY_TIMEOUT).map_err(db)?;
    conn.pragma_update("foreign_keys", "ON").await.map_err(db)?;
    conn.pragma_update("synchronous", "FULL")
        .await
        .map_err(db)?;
    verify_foreign_keys(conn).await
}

/// Read the pragma back: a build that ignored it would otherwise fail silently
/// at the first account delete.
async fn verify_foreign_keys(conn: &Connection) -> StoreResult<()> {
    let rows = exec::fetch_all(conn, "PRAGMA foreign_keys", Vec::new()).await?;
    let on = rows
        .first()
        .is_some_and(|row| matches!(row.get_value(0), Ok(::turso::Value::Integer(1))));
    if on {
        return Ok(());
    }
    Err(StoreError::InvalidConfig(
        "turso did not enable foreign_keys: account deletes would orphan Signal rows".into(),
    ))
}
