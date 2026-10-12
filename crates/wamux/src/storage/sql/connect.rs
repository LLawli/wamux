//! Connecting each driver. The pool options are per engine on purpose: the
//! engines differ in what a pool may safely do (see `connect_sqlite`).

use std::str::FromStr;
use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{ConnectOptions, PgPool};

use super::SqliteConn;
use super::sqlite_conn::SQLITE_READERS;
use crate::storage::group_commit::DEFAULT_BATCH_CAP;

/// Same patience on the writer and the readers.
const BUSY_TIMEOUT: Duration = Duration::from_secs(30);

/// Open a pooled connection to Postgres.
pub async fn connect_postgres(
    database_url: &str,
    max_connections: u32,
) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(database_url)
        .await
}

/// Open the database file as a writer plus a read-only pool (#173), with the
/// default batch cap.
///
/// Why two handles and not a pool: SQLite allows one writer at a time, and sqlx
/// opens transactions as BEGIN DEFERRED, so two connections that WRITE can hit
/// SQLITE_BUSY *without* honoring `busy_timeout`, which would surface as a random
/// store error under multi-account load. So exactly one connection writes (the
/// writer, behind the group commit), and the pool is `read_only`: its
/// connections never write, so that failure mode does not exist for them. The
/// writer opens the file first because a read-only connection cannot create it.
///
/// `synchronous = FULL` was decided in #169/#173, with the group commit paying
/// for it. The reference implementation's pragmas used NORMAL, which can lose
/// the last commits on power loss; here nothing acknowledged is lost.
///
/// `foreign_keys` is not cosmetic: SQLite defaults it OFF, and every store table
/// hangs off `accounts(device_id) ON DELETE CASCADE`. Without it, deleting an
/// account would silently orphan all of its Signal rows.
pub async fn connect_sqlite(database_url: &str) -> Result<SqliteConn, sqlx::Error> {
    connect_sqlite_with_batch_cap(database_url, DEFAULT_BATCH_CAP).await
}

/// `connect_sqlite` with the most jobs one COMMIT carries (#173): the bench
/// measures the serial baseline with a cap of 1.
pub async fn connect_sqlite_with_batch_cap(
    database_url: &str,
    batch_cap: usize,
) -> Result<SqliteConn, sqlx::Error> {
    let writer = writer_options(database_url)?.connect().await?;
    let readers = SqlitePoolOptions::new()
        .max_connections(SQLITE_READERS)
        .connect_with(reader_options(database_url)?)
        .await?;
    Ok(SqliteConn::new(writer, readers, batch_cap))
}

fn writer_options(database_url: &str) -> Result<SqliteConnectOptions, sqlx::Error> {
    Ok(SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full)
        .busy_timeout(BUSY_TIMEOUT)
        .foreign_keys(true))
}

/// No journal mode or synchronous here: those pragmas write, and the file is
/// already WAL by the time the readers open. `create_if_missing(false)` is
/// explicit because the test DSNs carry `?mode=rwc`.
fn reader_options(database_url: &str) -> Result<SqliteConnectOptions, sqlx::Error> {
    Ok(SqliteConnectOptions::from_str(database_url)?
        .read_only(true)
        .create_if_missing(false)
        .busy_timeout(BUSY_TIMEOUT))
}
