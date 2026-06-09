//! wamux entrypoint: load config, init observability, migrate, build the
//! account registry, load existing accounts (connect is edge-driven), bind the
//! socket, serve.

use std::sync::Arc;

use anyhow::Context;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

use wamux::config::Config;
use wamux::state::{AccountRegistry, RegistryTuning};
use wamux::{server, storage, transport};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::load().context("loading config")?;
    init_tracing(&config);

    let pool = storage::postgres::connect(&config.database_url, config.db_max_connections)
        .await
        .context("connecting to postgres")?;
    storage::postgres::run_migrations(&pool)
        .await
        .context("running migrations")?;

    let tuning = RegistryTuning {
        ring_capacity: config.event_ring_capacity,
        broadcast_capacity: config.broadcast_capacity,
        replay_max_event_bytes: config.replay_max_event_bytes,
        max_connected_accounts: config.max_connected_accounts,
        graceful_stop_timeout: Duration::from_millis(config.graceful_stop_timeout_ms),
        ws_url_override: None,
    };
    let registry = Arc::new(AccountRegistry::new(pool, tuning));
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

    let router = server::build_router(registry, &config);
    tracing::info!(socket = %config.socket_path, "wamux listening");

    router
        .serve_with_incoming_shutdown(stream, transport::shutdown::signal_future())
        .await
        .context("serving")?;

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
