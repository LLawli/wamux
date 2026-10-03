//! The one connection a `TursoStore` runs on (#106), and how it is opened.
//!
//! One `turso::Connection` behind a `tokio::sync::Mutex`, the equivalent of the
//! SQLite engine's `max_connections(1)`. Why one, measured on turso 0.8.1: a
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
use tokio::sync::{Mutex, MutexGuard};
use wacore::store::error::{Result as StoreResult, StoreError};

use super::exec;
use super::turso_error::db;

/// Same patience as the SQLite engine's `busy_timeout`.
const BUSY_TIMEOUT: Duration = Duration::from_secs(30);

/// The connection and the `Database` it came from. Field order is drop order:
/// the connection goes first, then the handle that closes the file.
struct ConnInner {
    conn: Mutex<Connection>,
    db: Database,
}

/// The one connection a `TursoStore` runs on, shared by every account.
#[derive(Clone)]
pub struct TursoConn(Arc<ConnInner>);

impl TursoConn {
    /// Open (creating if absent) the database file with the engine's pragmas.
    pub(super) async fn open(path: &Path) -> StoreResult<Self> {
        let path = path.to_str().ok_or_else(|| {
            StoreError::InvalidConfig(format!("turso path is not valid UTF-8: {path:?}"))
        })?;
        // `build` also converts the file header to WAL (the turso default).
        let database = Builder::new_local(path)
            .build()
            .await
            .map_err(|e| StoreError::Connection(Box::new(e)))?;
        let conn = database.connect().map_err(db)?;
        apply_pragmas(&conn).await?;
        let inner = ConnInner {
            conn: Mutex::new(conn),
            db: database,
        };
        Ok(Self(Arc::new(inner)))
    }

    /// Exclusive use of the connection until the guard drops.
    ///
    /// A transaction abandoned half way (an early `?`, or a request future
    /// dropped by a client that went away) leaves the connection inside it, and
    /// the next user would join it. Dropping is not async, so the cleanup is
    /// here, before anyone else is handed the connection: a ROLLBACK whenever
    /// the connection is not in autocommit.
    pub async fn lock(&self) -> MutexGuard<'_, Connection> {
        let guard = self.0.conn.lock().await;
        if matches!(guard.is_autocommit(), Ok(false)) {
            tracing::warn!("turso connection found inside an abandoned transaction, rolling back");
            if let Err(error) = guard.execute("ROLLBACK", ()).await {
                tracing::error!(%error, "rolling back an abandoned turso transaction failed");
            }
        }
        guard
    }

    /// Release the file before returning. Only possible when no other handle
    /// (a backend, a clone) is alive: they share the one connection.
    pub(super) async fn close(self) -> StoreResult<()> {
        let others = Arc::strong_count(&self.0) - 1;
        let inner = Arc::try_unwrap(self.0).map_err(|_| {
            StoreError::Connection(
                format!("turso store still in use by {others} other handle(s); drop them first")
                    .into(),
            )
        })?;
        let ConnInner { conn, db } = inner;
        drop(conn.into_inner());
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
