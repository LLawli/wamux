//! Connecting each driver. The pool options are per engine on purpose: the
//! engines differ in what a pool may safely do (see `connect_sqlite`).

use std::str::FromStr;
use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{PgPool, SqlitePool};

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

/// Open the database file, applying the reference implementation's pragmas.
///
/// `max_connections(1)` is deliberate and not a leftover. SQLite allows one
/// writer at a time, and sqlx opens transactions as BEGIN DEFERRED: two pooled
/// connections upgrading to a write inside a transaction can hit SQLITE_BUSY
/// *without* honoring `busy_timeout`, which would surface as a random store
/// error under multi-account load. A single connection makes the process
/// serialize its own writes instead, which is exactly what the whatsapp-rust
/// sqlite reference achieves with its 1-permit semaphore.
///
/// `foreign_keys` is likewise not cosmetic: SQLite defaults it OFF, and every
/// store table hangs off `accounts(device_id) ON DELETE CASCADE`. Without it,
/// deleting an account would silently orphan all of its Signal rows.
pub async fn connect_sqlite(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(30))
        .foreign_keys(true);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
}
