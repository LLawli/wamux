//! Storage integration tests: one binary for every suite that exercises the
//! store (engines, conversion, encryption, migrations, group commit), so they
//! link once instead of once each (#235).
//!
//! A new storage suite goes here as a `mod`. A suite that owns process-global
//! state (a global tracing subscriber or metrics recorder, the allocator, the
//! process umask, a peak-RSS measurement) must stay its own binary: in a shared
//! one it would see the other suites' state. `bincode_upgrade_notice` is one.

// Only a subset of the shared helpers is used per suite.
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;

mod appstate_bootstrap;
mod bincode_upgrade;
mod existing_store;
mod group_commit_sqlite;
mod group_commit_turso;
mod group_commit_turso_accounts;
mod isolated_pool;
mod migration_alignment;
mod sqlite_temp_store;
mod storage_backend;
mod store_cli;
mod store_conversion;
mod store_encryption;
mod store_file_mode;
mod store_parity;
mod store_rotation;
