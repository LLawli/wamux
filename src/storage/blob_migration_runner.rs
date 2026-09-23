//! The engine half of every one-shot blob migration: read each blob column,
//! plan every rewrite before touching anything, then write the whole plan in
//! one transaction. Shared by `migrate_0_7` and `migrate_0_7_main`, which differ
//! only in the `BlobMigrationSteps` they hand in.
//!
//! Moved here from `src/bin/migrate_0_7.rs` (#30) so the second migration does
//! not copy it. The contract it keeps:
//!
//! - **Plan first.** A single blob that fails to convert fails the run before a
//!   transaction is opened. "All or nothing" is cheap because of this.
//! - **One transaction.** A store half on the old layout and half on the new is
//!   worse than one still entirely on the old, because only the second re-runs.
//! - **Idempotent.** A blob already on the new layout is counted, not rewritten.
//! - **Counts only in the report.** A blob here is Signal key material.
//! - **Refuses while the daemon may be running** (unless `--force`): the daemon
//!   holds each `Device` in memory and writes it back on its own schedule.

use sqlx::{PgPool, SqlitePool};
use thiserror::Error;

use super::blob_migration::{BlobMigrationSteps, MigrateError};

/// Everything a run decided, before a byte is written.
#[derive(Debug, Default)]
pub struct MigrationPlan {
    /// `(device_id, new device.data)` for every row that needs a rewrite.
    pub devices: Vec<(i32, Vec<u8>)>,
    /// `(device_id, name, new state_data)` for every row that needs a rewrite.
    pub versions: Vec<(i32, String, Vec<u8>)>,
    pub devices_already_current: usize,
    pub versions_already_current: usize,
    pub sync_keys_verified: usize,
}

impl MigrationPlan {
    pub fn nothing_to_write(&self) -> bool {
        self.devices.is_empty() && self.versions.is_empty()
    }
}

#[derive(Debug, Error)]
pub enum RunError {
    /// A blob would not convert. Nothing was written.
    #[error(transparent)]
    Blob(#[from] MigrateError),
    /// The database refused a read or a write. `context` names which.
    #[error("{context}: {source}")]
    Database {
        context: String,
        #[source]
        source: sqlx::Error,
    },
}

/// Read all three blob columns of a Postgres store and plan every rewrite.
pub async fn build_plan_pg(
    pool: &PgPool,
    steps: &BlobMigrationSteps,
) -> Result<MigrationPlan, RunError> {
    let _ = (pool, steps);
    todo!("#30: move build_plan_pg from src/bin/migrate_0_7.rs, driven by `steps`")
}

/// Read all three blob columns of a SQLite store and plan every rewrite.
pub async fn build_plan_sqlite(
    pool: &SqlitePool,
    steps: &BlobMigrationSteps,
) -> Result<MigrationPlan, RunError> {
    let _ = (pool, steps);
    todo!("#30: move build_plan_sqlite from src/bin/migrate_0_7.rs, driven by `steps`")
}

/// Write the whole plan to a Postgres store in one transaction.
pub async fn apply_pg(pool: &PgPool, plan: &MigrationPlan) -> Result<(), RunError> {
    let _ = (pool, plan);
    todo!("#30: move apply_pg from src/bin/migrate_0_7.rs")
}

/// Write the whole plan to a SQLite store in one transaction.
pub async fn apply_sqlite(pool: &SqlitePool, plan: &MigrationPlan) -> Result<(), RunError> {
    let _ = (pool, plan);
    todo!("#30: move apply_sqlite from src/bin/migrate_0_7.rs")
}

/// The whole CLI of a migration bin: `[--apply] [--force] [--database-url URL]`,
/// config load, daemon check, plan, optional apply, report. Each bin's `main` is
/// this one call.
///
/// Returns `anyhow::Result` on purpose: this IS the body of a one-shot `main`,
/// the one place CLAUDE.md allows it.
pub async fn run_blob_migration_cli(steps: &BlobMigrationSteps) -> anyhow::Result<()> {
    let _ = steps;
    todo!("#30: move parse_args/main/report from src/bin/migrate_0_7.rs, naming `steps.name`")
}
