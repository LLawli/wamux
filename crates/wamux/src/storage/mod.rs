//! Persistence. Relay-pure: only whatsapp-rust's Signal/session/device state is
//! stored, never business message history.
//!
//! `StorageEngine` is the abstraction and the only plug point. Two families
//! implement wacore's store traits on a device-scoped backend type: `sql`
//! (Postgres and SQLite through sqlx) and `turso` (#106, the native turso crate,
//! behind the `turso` cargo feature). The statement text is written once, in
//! `statements`, and both families run it.

pub mod batch_chunks;
pub mod bincode_upgrade;
pub mod blob_codec;
/// PALLIATIVE for an upstream app-state bug; goes with #36.
pub mod engine;
pub(crate) mod protocol_rows;
pub mod sql;
pub mod sqlx_error;
pub mod statements;
#[cfg(feature = "turso")]
pub mod turso;

#[cfg(test)]
mod engine_dispatch_tests;

pub use engine::{AccountRow, StorageEngine};

use std::sync::Arc;

use wacore::store::error::StoreError;

/// Open the engine the DSN asks for, migrations applied, ready to inject.
///
/// The scheme picks the engine — there is no separate `storage_backend` knob,
/// so a config can never name one engine and point at the other's database.
/// `pg_max_connections` applies to Postgres only; the SQLite engine pins its
/// pool to one connection on purpose (see `sql::connect_sqlite`). `turso://`
/// needs the `turso` cargo feature (#106) and takes no options.
pub async fn open_engine(
    database_url: &str,
    pg_max_connections: u32,
) -> Result<Arc<dyn StorageEngine>, StoreError> {
    match dsn_scheme(database_url) {
        "postgres" | "postgresql" => Ok(Arc::new(
            sql::SqlStore::open_postgres(database_url, pg_max_connections).await?,
        )),
        "sqlite" => Ok(Arc::new(sql::SqlStore::open_sqlite(database_url).await?)),
        "turso" => open_turso(database_url).await,
        other => Err(StoreError::InvalidConfig(format!(
            "unsupported database_url scheme '{other}': expected one of \
             postgres://, postgresql://, sqlite://, turso://"
        ))),
    }
}

#[cfg(feature = "turso")]
async fn open_turso(database_url: &str) -> Result<Arc<dyn StorageEngine>, StoreError> {
    Ok(Arc::new(turso::TursoStore::open(database_url).await?))
}

/// Without the feature the scheme is still recognized, so the operator is told
/// what to rebuild with instead of getting "unsupported scheme" (#106).
#[cfg(not(feature = "turso"))]
async fn open_turso(database_url: &str) -> Result<Arc<dyn StorageEngine>, StoreError> {
    Err(StoreError::InvalidConfig(format!(
        "database_url '{database_url}' is a turso:// DSN, but this binary was compiled \
         without the `turso` feature: rebuild with `--features turso`"
    )))
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
