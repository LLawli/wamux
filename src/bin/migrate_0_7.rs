//! One-shot store migration for the whatsapp-rust 0.6 -> 0.7.0 upgrade.
//!
//! Without it, every paired account is lost: `device.data` and
//! `app_state_versions.state_data` are positional bincode blobs that gained a
//! field and no longer decode. See `storage::blob_migration_0_7` for the why and
//! the conversion itself; `storage::blob_migration_runner` is the CLI, the SQL,
//! and the guards, shared with `migrate_0_7_main`.
//!
//! ```text
//! cargo run --features migrate-0-7 --bin migrate_0_7            # dry run
//! cargo run --features migrate-0-7 --bin migrate_0_7 -- --apply # write
//! ```
//!
//! Its output is a 0.7.0 blob, so a store that started on 0.6 runs
//! `migrate_0_7_main` next. Dry run by default, one transaction, idempotent,
//! and it stops before writing anything if a single blob fails to convert.
//! Take a backup first anyway: a restore is the only recovery from a store this
//! touched.

use wamux::storage::blob_migration_0_7::MIGRATE_0_7;
use wamux::storage::blob_migration_runner::run_blob_migration_cli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run_blob_migration_cli(&MIGRATE_0_7).await
}
