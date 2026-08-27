//! The storage abstraction. One engine per deployment, picked from the
//! `database_url` scheme at startup.
//!
//! Two levels, deliberately: `wacore::store::traits::Backend` is per-ACCOUNT
//! (the four Signal/session/device traits the whatsapp-rust Bot consumes), and
//! it already abstracts the engine away. `StorageEngine` is per-DEPLOYMENT: it
//! owns the wamux-specific `accounts` table, applies migrations, and mints the
//! per-account `Backend`. Everything Postgres-shaped above the Bot lived in
//! `AccountRegistry` before this trait existed.

use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;
use wacore::store::error::Result as StoreResult;
use wacore::store::traits::Backend;

/// One row of the wamux-specific `accounts` table: canonical UUID and optional
/// `external_ref` mapped to the integer `device_id` that scopes every store
/// table. Engine-neutral — each engine decodes its own driver row into this
/// (Postgres stores `uuid` as UUID, SQLite as TEXT).
#[derive(Debug, Clone)]
pub struct AccountRow {
    pub uuid: Uuid,
    pub external_ref: Option<String>,
    pub device_id: i32,
    pub push_name: Option<String>,
    pub created_at: i64,
}

/// Account persistence plus a per-account `Backend` factory.
///
/// Kept to exactly what the registry and the health probe call. Engine-specific
/// extras (raw pool access for the stress bins, unused CRUD) stay inherent to
/// the concrete engine rather than widening this trait, so adding an engine
/// stays cheap.
#[async_trait]
pub trait StorageEngine: Send + Sync + 'static {
    /// Insert a new account. The engine assigns `device_id` (Postgres IDENTITY,
    /// SQLite AUTOINCREMENT), so it is never passed in.
    async fn create_account(&self, external_ref: Option<&str>) -> StoreResult<AccountRow>;

    /// Every persisted account, ordered by `device_id`. Called once at startup
    /// to rebuild the in-memory handles; connect stays edge-driven.
    async fn list_accounts(&self) -> StoreResult<Vec<AccountRow>>;

    /// Delete the account. The `ON DELETE CASCADE` on every scoped table wipes
    /// its Signal state with it — which in SQLite requires `foreign_keys=ON`,
    /// see the sqlite engine's connect options.
    async fn delete_account(&self, uuid: Uuid) -> StoreResult<bool>;

    /// wacore's storage backend scoped to one account's `device_id`. Cheap:
    /// implementations clone a pool handle, so this is called per connect.
    fn device_backend(&self, device_id: i32) -> Arc<dyn Backend>;

    /// Trivial round-trip for the readiness probe. Returns `false` rather than
    /// an error: AdminService wants `ready=false`, never a `Status`.
    async fn ping_storage(&self) -> bool;
}
