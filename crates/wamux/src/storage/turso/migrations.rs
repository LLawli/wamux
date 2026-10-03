//! Applying `migrations_sqlite/` on turso, recorded the way sqlx records them
//! (#106). The only file of the family that writes SQL itself: it owns sqlx's
//! `_sqlx_migrations` table, which no shared statement spells out.
//!
//! Why not just call `SQLITE_MIGRATOR.run`: it needs a sqlx connection, and a
//! second connection on a file turso has open contends for its lock. So the
//! runner reproduces what sqlx-sqlite 0.8 does (`migrate.rs`): the same table,
//! one transaction per migration holding the script and its bookkeeping row,
//! `execution_time` filled in afterwards, and the same refusals (a failed row,
//! a changed checksum, a version this build does not know). Description and
//! checksum come from the embedded migration itself, so a store migrated here
//! opens under sqlx, which rejects any checksum that differs, and the other way.
//!
//! A migration script is several statements, and `execute` runs only the first
//! of them and returns `Ok(0)` (measured on turso 0.8.1): scripts go through
//! `execute_batch`, one file per call.

use std::collections::HashMap;
use std::time::Instant;

use ::turso::{Connection, Value};
use sqlx::migrate::Migration;
use wacore::store::error::StoreError;

use super::TursoConn;
use super::exec::{self, binds};
use super::row_values::{blob, int};
use super::turso_error::db;
use crate::storage::sql::SQLITE_MIGRATOR;

/// sqlx-sqlite's own DDL, verbatim.
const CREATE_MIGRATIONS_TABLE: &str = "CREATE TABLE IF NOT EXISTS _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
)";
const SELECT_FAILED: &str =
    "SELECT version FROM _sqlx_migrations WHERE success = false ORDER BY version LIMIT 1";
const SELECT_APPLIED: &str = "SELECT version, checksum FROM _sqlx_migrations ORDER BY version";
const INSERT_APPLIED: &str =
    "INSERT INTO _sqlx_migrations ( version, description, success, checksum, execution_time )
    VALUES ( ?1, ?2, TRUE, ?3, -1 )";
const UPDATE_EXECUTION_TIME: &str =
    "UPDATE _sqlx_migrations SET execution_time = ?1 WHERE version = ?2";

#[derive(Debug, thiserror::Error)]
pub enum TursoMigrationError {
    #[error("migration {0} is recorded as failed (success = false); the store needs a manual fix")]
    Dirty(i64),
    #[error("migration {0} was applied with a different checksum than this build embeds")]
    ChecksumMismatch(i64),
    #[error("migration {0} is recorded in the store but unknown to this build")]
    VersionMissing(i64),
    #[error("migration {version} ({description}) failed")]
    Script {
        version: i64,
        description: String,
        #[source]
        source: ::turso::Error,
    },
    #[error("reading or writing _sqlx_migrations")]
    Bookkeeping(#[source] StoreError),
}

type MigrationResult<T> = Result<T, TursoMigrationError>;

/// Apply the migrations the store lacks, oldest first, under the one lock.
pub(super) async fn apply_pending(conn: &TursoConn) -> MigrationResult<()> {
    let conn = conn.lock().await;
    conn.execute(CREATE_MIGRATIONS_TABLE, ())
        .await
        .map_err(|e| bookkeeping(db(e)))?;
    reject_failed_row(&conn).await?;
    let applied = applied_checksums(&conn).await?;
    check_known(&applied)?;
    for migration in SQLITE_MIGRATOR.iter() {
        if migration.migration_type.is_down_migration() {
            continue;
        }
        match applied.get(&migration.version) {
            Some(checksum) => check_checksum(migration, checksum)?,
            None => apply_one(&conn, migration).await?,
        }
    }
    Ok(())
}

fn bookkeeping(source: StoreError) -> TursoMigrationError {
    TursoMigrationError::Bookkeeping(source)
}

async fn reject_failed_row(conn: &Connection) -> MigrationResult<()> {
    let failed = exec::fetch_optional(conn, SELECT_FAILED, binds![])
        .await
        .map_err(bookkeeping)?;
    match failed.map(|row| row.get_value(0)) {
        Some(Ok(Value::Integer(version))) => Err(TursoMigrationError::Dirty(version)),
        _ => Ok(()),
    }
}

async fn applied_checksums(conn: &Connection) -> MigrationResult<HashMap<i64, Vec<u8>>> {
    let rows = exec::fetch_all(conn, SELECT_APPLIED, binds![])
        .await
        .map_err(bookkeeping)?;
    let mut applied: HashMap<i64, Vec<u8>> = HashMap::new();
    for row in &rows {
        let version = int(row, 0).map_err(bookkeeping)?;
        let checksum = blob(row, 1).map_err(bookkeeping)?;
        applied.insert(version, checksum);
    }
    Ok(applied)
}

/// A version in the store that this build does not embed: a newer daemon wrote it.
fn check_known(applied: &HashMap<i64, Vec<u8>>) -> MigrationResult<()> {
    let known = |version: i64| SQLITE_MIGRATOR.iter().any(|m| m.version == version);
    match applied.keys().find(|version| !known(**version)) {
        Some(version) => Err(TursoMigrationError::VersionMissing(*version)),
        None => Ok(()),
    }
}

fn check_checksum(migration: &Migration, recorded: &[u8]) -> MigrationResult<()> {
    if migration.checksum.as_ref() == recorded {
        return Ok(());
    }
    Err(TursoMigrationError::ChecksumMismatch(migration.version))
}

/// One migration: its script and its bookkeeping row in one transaction, then
/// the elapsed time (lost if the process dies in between, as in sqlx).
async fn apply_one(conn: &Connection, migration: &Migration) -> MigrationResult<()> {
    let started = Instant::now();
    conn.execute("BEGIN", ())
        .await
        .map_err(|e| bookkeeping(db(e)))?;
    if let Err(error) = run_script_and_record(conn, migration).await {
        // A statement that fails inside BEGIN leaves the transaction open.
        let _ = conn.execute("ROLLBACK", ()).await;
        return Err(error);
    }
    conn.execute("COMMIT", ())
        .await
        .map_err(|e| bookkeeping(db(e)))?;
    let nanos = i64::try_from(started.elapsed().as_nanos()).unwrap_or(i64::MAX);
    let sql = UPDATE_EXECUTION_TIME;
    exec::execute(conn, sql, binds![nanos, migration.version])
        .await
        .map(drop)
        .map_err(bookkeeping)
}

async fn run_script_and_record(conn: &Connection, migration: &Migration) -> MigrationResult<()> {
    conn.execute_batch(migration.sql.as_ref())
        .await
        .map_err(|source| TursoMigrationError::Script {
            version: migration.version,
            description: migration.description.to_string(),
            source,
        })?;
    let checksum: Vec<u8> = migration.checksum.to_vec();
    let description: String = migration.description.to_string();
    exec::execute(
        conn,
        INSERT_APPLIED,
        binds![migration.version, description, checksum],
    )
    .await
    .map(drop)
    .map_err(bookkeeping)
}
