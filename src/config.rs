//! Configuration: loaded once at startup from a TOML file plus `WAMUX_` env
//! overrides (env wins). No hardcoded paths/endpoints live in services.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Unix socket path the gRPC server binds.
    pub socket_path: String,
    /// Socket file mode (octal, e.g. 0o660). Stored as decimal in TOML/env.
    pub socket_mode: u32,
    /// Optional owning group name to chown the socket to.
    pub socket_group: Option<String>,
    /// Database DSN. The scheme picks the storage engine:
    /// `postgres://` / `postgresql://` or `sqlite://` (file path; created if
    /// absent). Anything else fails at startup.
    pub database_url: String,
    /// Postgres pool size. Ignored by the SQLite engine, which pins its pool to
    /// a single connection to serialize writes (see `storage::sqlite::connect`).
    pub db_max_connections: u32,
    /// Per-account in-memory replay ring capacity.
    pub event_ring_capacity: usize,
    /// Per-account live broadcast channel capacity (slow subscribers lag past it
    /// and receive a `gap` marker; see `EventService`).
    pub broadcast_capacity: usize,
    /// Largest event (encoded bytes) still kept in the replay ring. 0 = no cap.
    /// History-sync blobs are excluded from the ring regardless of this.
    pub replay_max_event_bytes: u64,
    /// Max accounts the core will keep connected at once (fd/resource budget).
    /// 0 = unlimited. `Connect` past this returns `ResourceExhausted`.
    pub max_connected_accounts: usize,
    /// Grace period for a per-account graceful stop (`Client::disconnect` +
    /// awaiting the run loop) before falling back to a hard abort.
    pub graceful_stop_timeout_ms: u64,
    /// Max bytes accepted for an inbound media send (inline streamed chunks).
    pub media_max_bytes: u64,
    /// `tracing` env-filter directive.
    pub log_level: String,
    /// Log output format: "json" for structured one-line-per-event, anything
    /// else for the human-readable text formatter.
    pub log_format: String,
    /// Register gRPC reflection (dev tooling).
    pub enable_reflection: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            socket_path: "/run/wamux.sock".to_string(),
            socket_mode: 0o660,
            socket_group: None,
            database_url: "postgres://wamux:wamux@localhost:5432/wamux".to_string(),
            db_max_connections: 16,
            event_ring_capacity: 256,
            broadcast_capacity: 1024,
            replay_max_event_bytes: 0,
            max_connected_accounts: 200,
            graceful_stop_timeout_ms: 3000,
            media_max_bytes: 100 * 1024 * 1024,
            log_level: "info,wamux=debug".to_string(),
            log_format: "text".to_string(),
            enable_reflection: true,
        }
    }
}

impl Config {
    /// Load defaults <- TOML file (`WAMUX_CONFIG` or `wamux.toml`) <- `WAMUX_` env.
    pub fn load() -> anyhow::Result<Self> {
        use figment::Figment;
        use figment::providers::{Env, Format, Serialized, Toml};

        let path = std::env::var("WAMUX_CONFIG").unwrap_or_else(|_| "wamux.toml".to_string());
        let config: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::file(path))
            .merge(Env::prefixed("WAMUX_"))
            .extract()?;
        Ok(config)
    }
}
