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

use anyhow::{Context, bail};
use sqlx::{PgPool, SqlitePool};
use thiserror::Error;

use super::blob_migration::{BlobMigration, BlobMigrationSteps, MigrateError};
use crate::config::Config;
use crate::storage::{postgres, sqlite};

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

/// Convert one column's worth of rows, or fail the whole run. `migrate` returns
/// `AlreadyCurrent` for a row that needs nothing, which is what makes a re-run
/// after a partial failure safe.
fn plan_blob<T>(
    rows: Vec<(T, Vec<u8>)>,
    migrate: fn(&[u8]) -> Result<BlobMigration, MigrateError>,
    already_current: &mut usize,
) -> Result<Vec<(T, Vec<u8>)>, RunError> {
    let mut out = Vec::new();
    for (key, blob) in rows {
        match migrate(&blob)? {
            BlobMigration::AlreadyCurrent => *already_current += 1,
            BlobMigration::Rewritten(bytes) => out.push((key, bytes)),
        }
    }
    Ok(out)
}

/// Read all three blob columns of a Postgres store and plan every rewrite.
pub async fn build_plan_pg(
    pool: &PgPool,
    steps: &BlobMigrationSteps,
) -> Result<MigrationPlan, RunError> {
    let mut plan = MigrationPlan::default();

    let devices: Vec<(i32, Vec<u8>)> =
        sqlx::query_as("SELECT device_id, data FROM device ORDER BY device_id")
            .fetch_all(pool)
            .await
            .map_err(|source| db_error("reading device", source))?;
    plan.devices = plan_blob(devices, steps.device, &mut plan.devices_already_current)?;

    let versions: Vec<(i32, String, Vec<u8>)> = sqlx::query_as(
        "SELECT device_id, name, state_data FROM app_state_versions ORDER BY device_id, name",
    )
    .fetch_all(pool)
    .await
    .map_err(|source| db_error("reading app_state_versions", source))?;
    plan.versions = plan_versions(
        versions,
        steps.hash_state,
        &mut plan.versions_already_current,
    )?;

    let keys: Vec<(Vec<u8>,)> = sqlx::query_as("SELECT key_data FROM app_state_keys")
        .fetch_all(pool)
        .await
        .map_err(|source| db_error("reading app_state_keys", source))?;
    for (blob,) in &keys {
        (steps.sync_key)(blob)?;
    }
    plan.sync_keys_verified = keys.len();
    Ok(plan)
}

/// Read all three blob columns of a SQLite store and plan every rewrite.
pub async fn build_plan_sqlite(
    pool: &SqlitePool,
    steps: &BlobMigrationSteps,
) -> Result<MigrationPlan, RunError> {
    let mut plan = MigrationPlan::default();

    let devices: Vec<(i32, Vec<u8>)> =
        sqlx::query_as("SELECT device_id, data FROM device ORDER BY device_id")
            .fetch_all(pool)
            .await
            .map_err(|source| db_error("reading device", source))?;
    plan.devices = plan_blob(devices, steps.device, &mut plan.devices_already_current)?;

    let versions: Vec<(i32, String, Vec<u8>)> = sqlx::query_as(
        "SELECT device_id, name, state_data FROM app_state_versions ORDER BY device_id, name",
    )
    .fetch_all(pool)
    .await
    .map_err(|source| db_error("reading app_state_versions", source))?;
    plan.versions = plan_versions(
        versions,
        steps.hash_state,
        &mut plan.versions_already_current,
    )?;

    let keys: Vec<(Vec<u8>,)> = sqlx::query_as("SELECT key_data FROM app_state_keys")
        .fetch_all(pool)
        .await
        .map_err(|source| db_error("reading app_state_keys", source))?;
    for (blob,) in &keys {
        (steps.sync_key)(blob)?;
    }
    plan.sync_keys_verified = keys.len();
    Ok(plan)
}

/// `app_state_versions` is keyed by `(device_id, name)`, unlike `device`'s bare
/// `device_id`, so it goes through its own wrapper around `plan_blob` rather
/// than reusing the two-element key shape.
fn plan_versions(
    rows: Vec<(i32, String, Vec<u8>)>,
    migrate: fn(&[u8]) -> Result<BlobMigration, MigrateError>,
    already_current: &mut usize,
) -> Result<Vec<(i32, String, Vec<u8>)>, RunError> {
    let keyed: Vec<((i32, String), Vec<u8>)> = rows
        .into_iter()
        .map(|(device_id, name, blob)| ((device_id, name), blob))
        .collect();
    Ok(plan_blob(keyed, migrate, already_current)?
        .into_iter()
        .map(|((device_id, name), blob)| (device_id, name, blob))
        .collect())
}

/// Wraps a raw `sqlx::Error` with what was being attempted. A free function
/// rather than a closure factory: a closure capturing `context: &str` and
/// returning a `move` closure over it does not type-check across the several
/// call sites below (each borrow gets its own inferred lifetime, and nothing
/// unifies them), where a plain `fn` with an elided-per-call lifetime does.
fn db_error(context: &str, source: sqlx::Error) -> RunError {
    RunError::Database {
        context: context.to_string(),
        source,
    }
}

/// One transaction for the whole store: a store half on the old layout and
/// half on the new is worse than one still entirely on the old, because only
/// the second is re-runnable.
pub async fn apply_pg(pool: &PgPool, plan: &MigrationPlan) -> Result<(), RunError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|source| db_error("begin", source))?;
    for (device_id, blob) in &plan.devices {
        sqlx::query("UPDATE device SET data = $1 WHERE device_id = $2")
            .bind(blob)
            .bind(device_id)
            .execute(&mut *tx)
            .await
            .map_err(|source| db_error("updating device", source))?;
    }
    for (device_id, name, blob) in &plan.versions {
        sqlx::query(
            "UPDATE app_state_versions SET state_data = $1 WHERE device_id = $2 AND name = $3",
        )
        .bind(blob)
        .bind(device_id)
        .bind(name)
        .execute(&mut *tx)
        .await
        .map_err(|source| db_error("updating app_state_versions", source))?;
    }
    tx.commit()
        .await
        .map_err(|source| db_error("commit", source))
}

pub async fn apply_sqlite(pool: &SqlitePool, plan: &MigrationPlan) -> Result<(), RunError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|source| db_error("begin", source))?;
    for (device_id, blob) in &plan.devices {
        sqlx::query("UPDATE device SET data = ? WHERE device_id = ?")
            .bind(blob)
            .bind(device_id)
            .execute(&mut *tx)
            .await
            .map_err(|source| db_error("updating device", source))?;
    }
    for (device_id, name, blob) in &plan.versions {
        sqlx::query(
            "UPDATE app_state_versions SET state_data = ? WHERE device_id = ? AND name = ?",
        )
        .bind(blob)
        .bind(device_id)
        .bind(name)
        .execute(&mut *tx)
        .await
        .map_err(|source| db_error("updating app_state_versions", source))?;
    }
    tx.commit()
        .await
        .map_err(|source| db_error("commit", source))
}

struct Options {
    apply: bool,
    force: bool,
    database_url: Option<String>,
}

fn parse_args(bin_name: &str) -> anyhow::Result<Options> {
    let mut options = Options {
        apply: false,
        force: false,
        database_url: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--apply" => options.apply = true,
            "--force" => options.force = true,
            "--database-url" => {
                options.database_url = Some(args.next().context("--database-url needs a value")?);
            }
            "-h" | "--help" => {
                println!(
                    "usage: {bin_name} [--apply] [--force] [--database-url URL]\n\
                     \n\
                     Converts the whatsapp-rust bincode blobs in place.\n\
                     Without --apply it only reports what it would do.\n\
                     --force skips the running-daemon check (see below).\n"
                );
                std::process::exit(0);
            }
            other => bail!("unknown argument '{other}' (try --help)"),
        }
    }
    Ok(options)
}

/// The daemon keeps the decoded `Device` in memory and writes it back on its own
/// schedule, so a migration racing a live daemon is silently undone (or worse,
/// half-applied). The socket file is the cheapest evidence one is running.
fn refuse_while_the_daemon_may_be_running(socket_path: &str, force: bool) -> anyhow::Result<()> {
    if force || !std::path::Path::new(socket_path).exists() {
        return Ok(());
    }
    bail!(
        "'{socket_path}' exists, so a wamux daemon is probably running. It holds each \
         account's Device in memory and will overwrite whatever this tool writes. Stop \
         the daemon first, or pass --force if you are certain the socket is stale."
    )
}

/// Counts only. A blob here is Signal key material, so nothing about its content
/// belongs on a terminal or in a scrollback.
fn report(steps: &BlobMigrationSteps, plan: &MigrationPlan, applied: bool) {
    let verb = if applied { "migrated" } else { "would migrate" };
    println!("migration: {}", steps.name);
    println!(
        "device.data:                   {verb} {}, already current {}",
        plan.devices.len(),
        plan.devices_already_current
    );
    println!(
        "app_state_versions.state_data: {verb} {}, already current {}",
        plan.versions.len(),
        plan.versions_already_current
    );
    println!(
        "app_state_keys.key_data:       {} verified, none need rewriting",
        plan.sync_keys_verified
    );

    if plan.nothing_to_write() {
        println!("\nnothing to do: this store is already on the target blob format.");
    } else if applied {
        println!("\ncommitted. Start the daemon and confirm each account reconnects.");
    } else {
        println!("\ndry run: nothing was written. Re-run with --apply to commit.");
    }
}

/// The whole CLI of a migration bin: `[--apply] [--force] [--database-url URL]`,
/// config load, daemon check, plan, optional apply, report. Each bin's `main` is
/// this one call.
///
/// Returns `anyhow::Result` on purpose: this IS the body of a one-shot `main`,
/// the one place CLAUDE.md allows it.
pub async fn run_blob_migration_cli(steps: &BlobMigrationSteps) -> anyhow::Result<()> {
    let options = parse_args(steps.name)?;
    let config = Config::load().context("loading config")?;
    let database_url = options
        .database_url
        .clone()
        .unwrap_or_else(|| config.database_url.clone());

    refuse_while_the_daemon_may_be_running(&config.socket_path, options.force)?;

    let scheme = database_url
        .split_once(':')
        .map(|(scheme, _)| scheme)
        .unwrap_or(&database_url);

    // Opening the engine also applies the SQL migrations, which is the right
    // order: the schema the target layout needs must exist before the blobs move.
    let plan = match scheme {
        "postgres" | "postgresql" => {
            let store = postgres::PgStorage::open(&database_url, config.db_max_connections)
                .await
                .context("opening Postgres")?;
            let plan = build_plan_pg(store.pool(), steps).await?;
            if options.apply {
                apply_pg(store.pool(), &plan).await?;
            }
            plan
        }
        "sqlite" => {
            let store = sqlite::SqliteStorage::open(&database_url)
                .await
                .context("opening SQLite")?;
            let plan = build_plan_sqlite(store.pool(), steps).await?;
            if options.apply {
                apply_sqlite(store.pool(), &plan).await?;
            }
            plan
        }
        other => bail!("unsupported database_url scheme '{other}'"),
    };

    report(steps, &plan, options.apply);
    Ok(())
}
