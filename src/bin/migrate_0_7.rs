//! One-shot store migration for the whatsapp-rust 0.6 -> 0.7 upgrade.
//!
//! Without it, every paired account is lost: `device.data` and
//! `app_state_versions.state_data` are positional bincode blobs that gained a
//! field and no longer decode. See `storage::blob_migration_0_7` for the why and
//! the conversion itself; this binary is only the CLI, the SQL, and the guards.
//!
//! ```text
//! cargo run --features migrate-0-7 --bin migrate_0_7            # dry run
//! cargo run --features migrate-0-7 --bin migrate_0_7 -- --apply # write
//! ```
//!
//! Dry run by default, one transaction, idempotent, and it stops before writing
//! anything if a single blob fails to convert. Take a backup first anyway: a
//! restore is the only recovery from a store this touched.

use anyhow::{Context, Result, bail};
use sqlx::{PgPool, SqlitePool};
use wamux::config::Config;
use wamux::storage::blob_migration_0_7::{
    BlobMigration, migrate_device_blob, migrate_hash_state_blob, verify_sync_key_blob,
};
use wamux::storage::{postgres, sqlite};

/// Everything the run decided, before a byte is written. Building the whole plan
/// first is what makes "all or nothing" cheap: a failure anywhere means we never
/// opened a transaction.
#[derive(Default)]
struct MigrationPlan {
    devices: Vec<(i32, Vec<u8>)>,
    versions: Vec<(i32, String, Vec<u8>)>,
    devices_already_current: usize,
    versions_already_current: usize,
    sync_keys_verified: usize,
}

impl MigrationPlan {
    fn nothing_to_write(&self) -> bool {
        self.devices.is_empty() && self.versions.is_empty()
    }
}

struct Options {
    apply: bool,
    force: bool,
    database_url: Option<String>,
}

fn parse_args() -> Result<Options> {
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
                    "usage: migrate_0_7 [--apply] [--force] [--database-url URL]\n\
                     \n\
                     Converts the whatsapp-rust 0.6 bincode blobs to 0.7 in place.\n\
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
fn refuse_while_the_daemon_may_be_running(socket_path: &str, force: bool) -> Result<()> {
    if force || !std::path::Path::new(socket_path).exists() {
        return Ok(());
    }
    bail!(
        "'{socket_path}' exists, so a wamux daemon is probably running. It holds each \
         account's Device in memory and will overwrite whatever this tool writes. Stop \
         the daemon first, or pass --force if you are certain the socket is stale."
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    let options = parse_args()?;
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
    // order: the 0.7 schema (msg_secrets) must exist before the blobs move.
    let plan = match scheme {
        "postgres" | "postgresql" => {
            let store = postgres::PgStorage::open(&database_url, config.db_max_connections)
                .await
                .context("opening Postgres")?;
            let plan = build_plan_pg(store.pool()).await?;
            if options.apply {
                apply_pg(store.pool(), &plan).await?;
            }
            plan
        }
        "sqlite" => {
            let store = sqlite::SqliteStorage::open(&database_url)
                .await
                .context("opening SQLite")?;
            let plan = build_plan_sqlite(store.pool()).await?;
            if options.apply {
                apply_sqlite(store.pool(), &plan).await?;
            }
            plan
        }
        other => bail!("unsupported database_url scheme '{other}'"),
    };

    report(&plan, options.apply);
    Ok(())
}

/// Counts only. A blob here is Signal key material, so nothing about its content
/// belongs on a terminal or in a scrollback.
fn report(plan: &MigrationPlan, applied: bool) {
    let verb = if applied { "migrated" } else { "would migrate" };
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
        println!("\nnothing to do: this store is already on the 0.7 blob format.");
    } else if applied {
        println!("\ncommitted. Start the daemon and confirm each account reconnects.");
    } else {
        println!("\ndry run: nothing was written. Re-run with --apply to commit.");
    }
}

/// Convert one column's worth of rows, or fail the whole run. `migrate` returns
/// `AlreadyCurrent` for a row that needs nothing, which is what makes a re-run
/// after a partial failure safe.
fn plan_blob<T>(
    rows: Vec<(T, Vec<u8>)>,
    migrate: fn(&[u8]) -> Result<BlobMigration, wamux::storage::blob_migration_0_7::MigrateError>,
    already_current: &mut usize,
) -> Result<Vec<(T, Vec<u8>)>> {
    let mut out = Vec::new();
    for (key, blob) in rows {
        match migrate(&blob)? {
            BlobMigration::AlreadyCurrent => *already_current += 1,
            BlobMigration::Rewritten(bytes) => out.push((key, bytes)),
        }
    }
    Ok(out)
}

async fn build_plan_pg(pool: &PgPool) -> Result<MigrationPlan> {
    let mut plan = MigrationPlan::default();

    let devices: Vec<(i32, Vec<u8>)> =
        sqlx::query_as("SELECT device_id, data FROM device ORDER BY device_id")
            .fetch_all(pool)
            .await
            .context("reading device")?;
    plan.devices = plan_blob(
        devices,
        migrate_device_blob,
        &mut plan.devices_already_current,
    )?;

    let versions: Vec<(i32, String, Vec<u8>)> = sqlx::query_as(
        "SELECT device_id, name, state_data FROM app_state_versions ORDER BY device_id, name",
    )
    .fetch_all(pool)
    .await
    .context("reading app_state_versions")?;
    let keyed: Vec<((i32, String), Vec<u8>)> = versions
        .into_iter()
        .map(|(device_id, name, blob)| ((device_id, name), blob))
        .collect();
    plan.versions = plan_blob(
        keyed,
        migrate_hash_state_blob,
        &mut plan.versions_already_current,
    )?
    .into_iter()
    .map(|((device_id, name), blob)| (device_id, name, blob))
    .collect();

    let keys: Vec<(Vec<u8>,)> = sqlx::query_as("SELECT key_data FROM app_state_keys")
        .fetch_all(pool)
        .await
        .context("reading app_state_keys")?;
    for (blob,) in &keys {
        verify_sync_key_blob(blob)?;
    }
    plan.sync_keys_verified = keys.len();
    Ok(plan)
}

async fn build_plan_sqlite(pool: &SqlitePool) -> Result<MigrationPlan> {
    let mut plan = MigrationPlan::default();

    let devices: Vec<(i32, Vec<u8>)> =
        sqlx::query_as("SELECT device_id, data FROM device ORDER BY device_id")
            .fetch_all(pool)
            .await
            .context("reading device")?;
    plan.devices = plan_blob(
        devices,
        migrate_device_blob,
        &mut plan.devices_already_current,
    )?;

    let versions: Vec<(i32, String, Vec<u8>)> = sqlx::query_as(
        "SELECT device_id, name, state_data FROM app_state_versions ORDER BY device_id, name",
    )
    .fetch_all(pool)
    .await
    .context("reading app_state_versions")?;
    let keyed: Vec<((i32, String), Vec<u8>)> = versions
        .into_iter()
        .map(|(device_id, name, blob)| ((device_id, name), blob))
        .collect();
    plan.versions = plan_blob(
        keyed,
        migrate_hash_state_blob,
        &mut plan.versions_already_current,
    )?
    .into_iter()
    .map(|((device_id, name), blob)| (device_id, name, blob))
    .collect();

    let keys: Vec<(Vec<u8>,)> = sqlx::query_as("SELECT key_data FROM app_state_keys")
        .fetch_all(pool)
        .await
        .context("reading app_state_keys")?;
    for (blob,) in &keys {
        verify_sync_key_blob(blob)?;
    }
    plan.sync_keys_verified = keys.len();
    Ok(plan)
}

/// One transaction for the whole store: a store half on 0.6 and half on 0.7 is
/// worse than one still entirely on 0.6, because only the second is re-runnable.
async fn apply_pg(pool: &PgPool, plan: &MigrationPlan) -> Result<()> {
    let mut tx = pool.begin().await.context("begin")?;
    for (device_id, blob) in &plan.devices {
        sqlx::query("UPDATE device SET data = $1 WHERE device_id = $2")
            .bind(blob)
            .bind(device_id)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("updating device {device_id}"))?;
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
        .with_context(|| format!("updating app_state_versions {device_id}/{name}"))?;
    }
    tx.commit().await.context("commit")
}

async fn apply_sqlite(pool: &SqlitePool, plan: &MigrationPlan) -> Result<()> {
    let mut tx = pool.begin().await.context("begin")?;
    for (device_id, blob) in &plan.devices {
        sqlx::query("UPDATE device SET data = ? WHERE device_id = ?")
            .bind(blob)
            .bind(device_id)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("updating device {device_id}"))?;
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
        .with_context(|| format!("updating app_state_versions {device_id}/{name}"))?;
    }
    tx.commit().await.context("commit")
}
