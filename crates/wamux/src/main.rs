//! wamux entrypoint: load config, init observability, migrate, build the
//! account registry, load existing accounts (connect is edge-driven), bind the
//! socket, serve.

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Context;
use nix::sys::stat::{Mode, umask};
use std::time::Duration;
use tracing_subscriber::EnvFilter;

use wamux::cli::{self, Command};
use wamux::config::Config;
use wamux::state::{AccountRegistry, RegistryTuning};
use wamux::storage::{StoreKey, store_key::key_id_hex};
use wamux::{server, spool, storage, transport};

// musl's malloc is the slow part of the static build: measured in steady-state
// message load (100 accounts x 100 texts), it costs +33% CPU per message on
// Postgres and +70% on SQLite against glibc; mimalloc brings that within 2% and
// 10%. The gnu build keeps glibc's allocator (#218).
#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Umask applied to everything the daemon creates (#75): the store holds every
/// account's keys, so a file or directory made without an explicit mode must be
/// owner-only. Set by the process itself so no entrypoint shell is needed (a
/// scratch image has none) and a run outside systemd gets it too.
const DAEMON_UMASK: u32 = 0o077;

/// Applied first in `main`, before any file or directory is created.
fn restrict_umask() {
    umask(Mode::from_bits_truncate(DAEMON_UMASK));
}

/// The store key named by `store_key_file`, if any. Logs the key id (a hash
/// prefix, not the key) so an operator can tell which key is in use.
fn load_store_key(config: &Config) -> anyhow::Result<Option<StoreKey>> {
    let Some(path) = &config.store_key_file else {
        return Ok(None);
    };
    let key = StoreKey::from_file(Path::new(path))
        .with_context(|| format!("loading the store key from {path}"))?;
    tracing::info!(key_id = %key_id_hex(&key.id()), "store key loaded");
    Ok(Some(key))
}

/// `wamux` with no arguments serves; the other commands are one-shot (#165).
/// A usage error exits 2, any other failure 1 (the `Err` `main` returns),
/// success 0.
#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    restrict_umask();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse(&args) {
        Err(usage_error) => {
            eprintln!("{usage_error}");
            Ok(ExitCode::from(2))
        }
        Ok(Command::Help) => {
            println!("{}", cli::usage());
            Ok(ExitCode::SUCCESS)
        }
        Ok(Command::Serve) => serve().await.map(|()| ExitCode::SUCCESS),
        Ok(Command::StoreDecrypt { yes }) => store_decrypt(yes).await.map(|()| ExitCode::SUCCESS),
        Ok(Command::StoreRotateKey { new_key_file }) => store_rotate_key(&new_key_file)
            .await
            .map(|()| ExitCode::SUCCESS),
    }
}

/// `wamux store decrypt --yes` (#165). The confirmation is checked first: this
/// writes every account's keys to the store in the clear, and an operator who
/// typed the command half way should be told that, not surprised by it.
async fn store_decrypt(yes: bool) -> anyhow::Result<()> {
    if !yes {
        anyhow::bail!(
            "`wamux store decrypt` writes the keys of every account to the store in the clear: \
             anyone who can read the database or a backup of it can then impersonate the \
             accounts. Run `wamux store decrypt --yes` to confirm"
        );
    }
    let config = Config::load().context("loading config")?;
    init_tracing(&config);
    let Some(key) = load_store_key(&config)? else {
        anyhow::bail!(
            "`wamux store decrypt` needs store_key_file (WAMUX_STORE_KEY_FILE) set to the key \
             that encrypted the store"
        );
    };
    let decrypted = storage::decrypt_engine(&config.database_url, config.db_max_connections, &key)
        .await
        .context("decrypting the store")?;
    println!("decrypted {decrypted} account(s)");
    Ok(())
}

/// `wamux store rotate-key --new-key-file <path>` (#166).
async fn store_rotate_key(new_key_file: &Path) -> anyhow::Result<()> {
    let config = Config::load().context("loading config")?;
    init_tracing(&config);
    let Some(old) = load_store_key(&config)? else {
        anyhow::bail!(
            "`wamux store rotate-key` needs store_key_file (WAMUX_STORE_KEY_FILE) set to the \
             key the store is encrypted with now"
        );
    };
    let path = new_key_file.display();
    let new = StoreKey::from_file(new_key_file)
        .with_context(|| format!("loading the new store key from {path}"))?;
    let rotated =
        storage::rotate_engine(&config.database_url, config.db_max_connections, &old, &new)
            .await
            .context("rotating the store key")?;
    println!(
        "rotated {rotated} account(s) to key id {}",
        key_id_hex(&new.id())
    );
    Ok(())
}

async fn serve() -> anyhow::Result<()> {
    let config = Config::load().context("loading config")?;
    init_tracing(&config);

    // The key file is read before the store opens, and a bad one stops startup
    // here with a message that names the file, never the key (#164).
    let store_key = load_store_key(&config)?;

    // A second daemon stops here, before the migrations below can touch the
    // live one's store (#77). The bind probes again for the race.
    transport::uds_listener::refuse_if_served(&config.socket_path)
        .context("checking the socket path")?;

    // Media transfers are staged on disk (#79). A spool that cannot be written
    // fails here, naming the path, not on the first SendMedia.
    spool::prepare_spool_dir(Path::new(&config.media_spool_dir))
        .context("preparing the media spool directory")?;

    // The DSN scheme picks the engine (postgres://, sqlite://, or turso:// in a
    // `turso` build).
    let engine = storage::open_engine_keyed(
        &config.database_url,
        config.db_max_connections,
        store_key.as_ref(),
    )
    .await
    .context("opening storage")?;
    // After the open: the store's verifier has accepted the key by now, so this
    // line is true (a wrong key stops at the open above).
    if let Some(key) = &store_key {
        tracing::info!(key_id = %key_id_hex(&key.id()), "store encryption on");
    }

    let tuning = RegistryTuning {
        ring_capacity: config.event_ring_capacity,
        broadcast_capacity: config.broadcast_capacity,
        replay_max_event_bytes: config.replay_max_event_bytes,
        max_connected_accounts: config.max_connected_accounts,
        graceful_stop_timeout: Duration::from_millis(config.graceful_stop_timeout_ms),
        ws_url_override: None,
    };
    let registry = Arc::new(AccountRegistry::new(engine, tuning));
    registry
        .load_existing()
        .await
        .context("loading existing accounts")?;

    let stream = transport::uds_listener::bind(
        &config.socket_path,
        config.socket_mode,
        config.socket_group.as_deref(),
    )
    .context("binding unix socket")?;

    let shutdown = transport::shutdown::Shutdown::new();
    tokio::spawn({
        let shutdown = shutdown.clone();
        async move {
            transport::shutdown::signal_future().await;
            shutdown.trigger();
        }
    });

    let router = server::build_router(registry.clone(), &config, shutdown.clone());
    tracing::info!(socket = %config.socket_path, "wamux listening");

    let grace = Duration::from_millis(config.shutdown_grace_ms);
    let drained = server::serve_until_shutdown(router, stream, shutdown, grace)
        .await
        .context("serving")?;
    if drained == server::Drained::GraceElapsed {
        tracing::warn!(
            ?grace,
            "shutdown grace elapsed; closed the connections still open"
        );
    }

    // Stop the accounts before the socket goes (#35): the library flushes and
    // closes its transport instead of dying mid-write with the process.
    registry.stop_all().await;
    transport::uds_listener::unlink(&config.socket_path);
    tracing::info!("wamux stopped");
    Ok(())
}

fn init_tracing(config: &Config) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(config.log_level.clone()));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    // JSON for ingestion, text for humans (default). One line per event either way.
    if config.log_format == "json" {
        builder.json().init();
    } else {
        builder.init();
    }
}
