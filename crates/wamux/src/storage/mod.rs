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
pub mod blob_cipher;
pub mod blob_codec;
pub(crate) mod convert;
/// PALLIATIVE for an upstream app-state bug; goes with #36.
pub mod engine;
pub(crate) mod file_mode;
pub mod group_commit;
pub(crate) mod protocol_rows;
pub mod sealed_columns;
pub mod sql;
pub mod sqlx_error;
pub mod statements;
pub(crate) mod store_encryption;
pub mod store_key;
#[cfg(feature = "turso")]
pub mod turso;

#[cfg(test)]
mod engine_dispatch_tests;

pub use blob_cipher::{BlobCipher, BlobContext};
pub use engine::{AccountRow, StorageEngine};
pub use group_commit::CommitStats;
pub use store_key::{StoreKey, StoreKeyError};

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
    open_engine_keyed(database_url, pg_max_connections, None).await
}

/// Turn an encrypted store back into plaintext (#165): `wamux store decrypt`.
/// Opens the store WITHOUT resolving a cipher (it must open an encrypted store
/// the daemon would refuse to serve half way), requires the key to match the
/// store's verifier, and decrypts account by account, one transaction each,
/// resuming an interrupted run. Returns how many accounts it decrypted in this
/// run. The daemon must be stopped: nothing else may use the store meanwhile.
pub async fn decrypt_engine(
    database_url: &str,
    pg_max_connections: u32,
    key: &StoreKey,
) -> Result<usize, StoreError> {
    match dsn_scheme(database_url) {
        "postgres" | "postgresql" => {
            sql::SqlStore::decrypt_postgres(database_url, pg_max_connections, key).await
        }
        "sqlite" => sql::SqlStore::decrypt_sqlite(database_url, key).await,
        "turso" => decrypt_turso(database_url, key).await,
        other => Err(StoreError::InvalidConfig(format!(
            "unsupported database_url scheme '{other}': expected one of \
             postgres://, postgresql://, sqlite://, turso://"
        ))),
    }
}

/// Re-seal every blob of an encrypted store under a new key (#166): `wamux
/// store rotate-key`. Opens the store without serving it, checks `old` against
/// the store's verifier, turns every account in ONE transaction (an interrupted
/// run leaves the store whole under `old`), replaces the store's key id and
/// verifier, then scrubs the old blobs out of the file. Returns how many
/// accounts it rotated. The daemon must be stopped.
pub async fn rotate_engine(
    database_url: &str,
    pg_max_connections: u32,
    old: &StoreKey,
    new: &StoreKey,
) -> Result<usize, StoreError> {
    match dsn_scheme(database_url) {
        "postgres" | "postgresql" => {
            sql::SqlStore::rotate_postgres(database_url, pg_max_connections, old, new).await
        }
        "sqlite" => sql::SqlStore::rotate_sqlite(database_url, old, new).await,
        "turso" => rotate_turso(database_url, old, new).await,
        other => Err(StoreError::InvalidConfig(format!(
            "unsupported database_url scheme '{other}': expected one of \
             postgres://, postgresql://, sqlite://, turso://"
        ))),
    }
}

#[cfg(feature = "turso")]
async fn rotate_turso(
    database_url: &str,
    old: &StoreKey,
    new: &StoreKey,
) -> Result<usize, StoreError> {
    turso::TursoStore::rotate(database_url, old, new).await
}

#[cfg(not(feature = "turso"))]
async fn rotate_turso(
    database_url: &str,
    _old: &StoreKey,
    _new: &StoreKey,
) -> Result<usize, StoreError> {
    Err(turso_feature_missing(database_url))
}

#[cfg(feature = "turso")]
async fn decrypt_turso(database_url: &str, key: &StoreKey) -> Result<usize, StoreError> {
    turso::TursoStore::decrypt(database_url, key).await
}

#[cfg(not(feature = "turso"))]
async fn decrypt_turso(database_url: &str, _key: &StoreKey) -> Result<usize, StoreError> {
    Err(turso_feature_missing(database_url))
}

/// `open_engine` with the store key (#164): the engine resolves it against the
/// store's `store_encryption` state before any account is touched.
pub async fn open_engine_keyed(
    database_url: &str,
    pg_max_connections: u32,
    key: Option<&StoreKey>,
) -> Result<Arc<dyn StorageEngine>, StoreError> {
    match dsn_scheme(database_url) {
        "postgres" | "postgresql" => Ok(Arc::new(
            sql::SqlStore::open_postgres_keyed(database_url, pg_max_connections, key).await?,
        )),
        "sqlite" => Ok(Arc::new(
            sql::SqlStore::open_sqlite_keyed(database_url, key).await?,
        )),
        "turso" => open_turso(database_url, key).await,
        other => Err(StoreError::InvalidConfig(format!(
            "unsupported database_url scheme '{other}': expected one of \
             postgres://, postgresql://, sqlite://, turso://"
        ))),
    }
}

#[cfg(feature = "turso")]
async fn open_turso(
    database_url: &str,
    key: Option<&StoreKey>,
) -> Result<Arc<dyn StorageEngine>, StoreError> {
    Ok(Arc::new(
        turso::TursoStore::open_keyed(database_url, key).await?,
    ))
}

/// Without the feature the scheme is still recognized, so the operator is told
/// what to rebuild with instead of getting "unsupported scheme" (#106).
#[cfg(not(feature = "turso"))]
async fn open_turso(
    database_url: &str,
    _key: Option<&StoreKey>,
) -> Result<Arc<dyn StorageEngine>, StoreError> {
    Err(turso_feature_missing(database_url))
}

#[cfg(not(feature = "turso"))]
fn turso_feature_missing(database_url: &str) -> StoreError {
    StoreError::InvalidConfig(format!(
        "database_url '{database_url}' is a turso:// DSN, but this binary was compiled \
         without the `turso` feature: rebuild with `--features turso`"
    ))
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
