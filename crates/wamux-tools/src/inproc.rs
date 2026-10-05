//! In-process bootstrap for the binaries that run an account without the
//! daemon (`pair_cli`, `pair_backfill`, `stress_live`): Postgres storage, a
//! registry with every persisted account loaded, and resolve-or-create (#64).

use std::sync::Arc;

use anyhow::Context as _;
use tracing_subscriber::EnvFilter;
use wamux::state::{AccountHandle, AccountRegistry, RegistryTuning};
use wamux::storage::sql::SqlStore;
use wamux_types::{AccountRef, ExternalRef};

use crate::live_env::EnvLookup;

pub const DATABASE_URL_VAR: &str = "DATABASE_URL";
/// The dockerized dev database from CLAUDE.md.
pub const DEFAULT_DATABASE_URL: &str = "postgres://wamux:wamux@localhost:5433/wamux";

/// `DATABASE_URL`, else the dev database.
pub fn database_url_from(lookup: EnvLookup) -> String {
    lookup(DATABASE_URL_VAR).unwrap_or_else(|| DEFAULT_DATABASE_URL.to_string())
}

/// `tracing` to stderr, filtered by `RUST_LOG` or `default_filter`.
pub fn init_tracing(default_filter: &str) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

/// Open (and migrate) the Postgres store at `database_url`. Callers that need
/// the raw pool (`stress_live`) take it from `SqlStore::pool`.
pub async fn open_engine(database_url: &str) -> anyhow::Result<Arc<SqlStore>> {
    let engine = SqlStore::open_postgres(database_url, 16)
        .await
        .context("opening the Postgres store")?;
    Ok(Arc::new(engine))
}

/// A registry over `engine` with the given tuning and every persisted
/// account loaded.
pub async fn load_registry(
    engine: Arc<SqlStore>,
    tuning: RegistryTuning,
) -> anyhow::Result<Arc<AccountRegistry>> {
    let registry = Arc::new(AccountRegistry::new(engine, tuning));
    registry
        .load_existing()
        .await
        .context("loading persisted accounts")?;
    Ok(registry)
}

/// Open (and migrate) the Postgres store at `database_url` and load every
/// persisted account into a fresh registry.
pub async fn open_registry(database_url: &str) -> anyhow::Result<Arc<AccountRegistry>> {
    let engine = open_engine(database_url).await?;
    load_registry(engine, RegistryTuning::with_ring(256)).await
}

/// The account named `external_ref`, created when it does not exist yet.
/// The bool is true when it was created (a fresh account needs pairing).
pub async fn resolve_or_create(
    registry: &Arc<AccountRegistry>,
    external_ref: &str,
) -> anyhow::Result<(Arc<AccountHandle>, bool)> {
    if let Ok(handle) = registry.resolve(&AccountRef::External(ExternalRef::new(external_ref))) {
        return Ok((handle, false));
    }
    let handle = registry
        .create_account(Some(external_ref))
        .await
        .with_context(|| format!("creating account {external_ref}"))?;
    Ok((handle, true))
}
