//! One-shot store migration for the whatsapp-rust 0.7.0 -> git main bump (#30).
//!
//! Without it, every paired account is lost: `device.data` and
//! `app_state_versions.state_data` are positional bincode blobs whose layout
//! changed. See `storage::blob_migration_0_7_main` for the conversion and
//! `storage::blob_migration_runner` for the guards.
//!
//! ```text
//! cargo run --features migrate-0-7-main --bin migrate_0_7_main            # dry run
//! cargo run --features migrate-0-7-main --bin migrate_0_7_main -- --apply # write
//! ```
//!
//! A store still on 0.6 runs `migrate_0_7` first, then this. Take a backup
//! first anyway: a restore is the only recovery from a store this touched.

use wamux::storage::blob_migration_0_7_main::MIGRATE_0_7_MAIN;
use wamux::storage::blob_migration_runner::run_blob_migration_cli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run_blob_migration_cli(&MIGRATE_0_7_MAIN).await
}
