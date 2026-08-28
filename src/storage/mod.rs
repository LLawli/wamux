//! Persistence. Relay-pure: only whatsapp-rust's Signal/session/device state is
//! stored, never business message history.
//!
//! `StorageEngine` is the abstraction; `postgres` and `sqlite` are its two
//! implementations, each implementing wacore's four store traits on a
//! device-scoped backend type.

pub mod blob_codec;
/// 0.6 -> 0.7 bincode blob conversion. Compiled only under `migrate-0-7`, which
/// is what links the second `wacore`; delete both once every store has run it.
#[cfg(feature = "migrate-0-7")]
pub mod blob_migration_0_7;
pub mod engine;
pub mod postgres;
pub mod sqlite;
pub mod sqlx_error;

pub use engine::{AccountRow, StorageEngine};

use std::sync::Arc;

use wacore::store::error::StoreError;

/// Open the engine the DSN asks for, migrations applied, ready to inject.
///
/// The scheme picks the engine — there is no separate `storage_backend` knob,
/// so a config can never name one engine and point at the other's database.
/// `pg_max_connections` applies to Postgres only; the SQLite engine pins its
/// pool to one connection on purpose (see `sqlite::connect`).
pub async fn open_engine(
    database_url: &str,
    pg_max_connections: u32,
) -> Result<Arc<dyn StorageEngine>, StoreError> {
    match dsn_scheme(database_url) {
        "postgres" | "postgresql" => Ok(Arc::new(
            postgres::PgStorage::open(database_url, pg_max_connections).await?,
        )),
        "sqlite" => Ok(Arc::new(sqlite::SqliteStorage::open(database_url).await?)),
        other => Err(StoreError::InvalidConfig(format!(
            "unsupported database_url scheme '{other}': expected one of \
             postgres://, postgresql://, sqlite://"
        ))),
    }
}

/// The scheme of a DSN: everything before the first `:`. Returns the whole
/// string when there is no `:` at all, so the error message can quote it.
fn dsn_scheme(database_url: &str) -> &str {
    database_url
        .split_once(':')
        .map(|(scheme, _)| scheme)
        .unwrap_or(database_url)
}

#[cfg(test)]
mod dsn_tests {
    use super::dsn_scheme;

    #[test]
    fn scheme_is_read_from_both_dsn_shapes() {
        // sqlx accepts sqlite with and without the authority slashes.
        assert_eq!(
            dsn_scheme("postgres://wamux:pw@localhost:5433/wamux"),
            "postgres"
        );
        assert_eq!(
            dsn_scheme("sqlite:///var/lib/wamux/wamux.db?mode=rwc"),
            "sqlite"
        );
        assert_eq!(dsn_scheme("sqlite:wamux.db"), "sqlite");
    }

    #[test]
    fn a_bare_path_yields_itself_so_the_error_can_quote_it() {
        assert_eq!(
            dsn_scheme("/var/lib/wamux/wamux.db"),
            "/var/lib/wamux/wamux.db"
        );
    }
}
