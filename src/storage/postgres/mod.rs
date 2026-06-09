//! Postgres implementation of wacore's storage backend.
//!
//! `wacore::store::traits::Backend` is a blanket impl over the four domain
//! traits (`SignalStore`, `AppSyncStore`, `ProtocolStore`, `DeviceStore`), so
//! implementing all four on `PgBackend` makes it a `Backend`.
//!
//! Multi-tenancy: one shared `PgPool`, one `PgBackend` instance per account,
//! each carrying the integer `device_id` that scopes every row. Byte formats
//! match the sqlite reference exactly (see docs/crate-notes/sqlite-reference.md).

mod accounts;
mod app_sync_store;
mod device_store;
mod error_map;
mod protocol_store;
mod signal_store;

pub use accounts::{AccountRow, Accounts};

use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use wacore::store::error::StoreError;

/// Embedded migrations (compiled in; no DB needed at build time).
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// A storage backend bound to a single account's `device_id`.
#[derive(Clone)]
pub struct PgBackend {
    pub(crate) pool: PgPool,
    pub(crate) device_id: i32,
}

impl PgBackend {
    pub fn new(pool: PgPool, device_id: i32) -> Self {
        Self { pool, device_id }
    }

    pub fn device_id(&self) -> i32 {
        self.device_id
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

/// Open a pooled connection to Postgres.
pub async fn connect(database_url: &str, max_connections: u32) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(database_url)
        .await
}

/// Apply pending migrations.
pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    MIGRATOR.run(pool).await
}

/// bincode-standard encode, matching the reference's structured-blob columns
/// (`app_state_keys.key_data`, `app_state_versions.state_data`, `device.data`).
pub(crate) fn bincode_encode<T: Serialize>(value: &T) -> Result<Vec<u8>, StoreError> {
    bincode::serde::encode_to_vec(value, bincode::config::standard())
        .map_err(|e| StoreError::Serialization(Box::new(e)))
}

/// bincode-standard decode counterpart.
pub(crate) fn bincode_decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, StoreError> {
    bincode::serde::decode_from_slice(bytes, bincode::config::standard())
        .map(|(value, _)| value)
        .map_err(|e| StoreError::Serialization(Box::new(e)))
}

/// Unix seconds, matching the reference's `wacore::time::now_secs()` semantics.
pub(crate) fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
