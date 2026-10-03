//! A real wamux daemon for the wamux-tools suites (#64): the production router
//! on a throwaway Unix socket, over a fresh SQLite store, so these tests need
//! no Postgres and touch no WhatsApp account.

use std::path::PathBuf;
use std::sync::Arc;

use wamux::config::Config;
use wamux::state::{AccountRegistry, RegistryTuning};
use wamux::storage::sql::SqlStore;
use wamux::{server, transport};

pub struct TestDaemon {
    pub socket: PathBuf,
    pub registry: Arc<AccountRegistry>,
    // Holds the socket and the database file; dropped with the daemon.
    _dir: tempfile::TempDir,
}

impl TestDaemon {
    pub async fn start() -> TestDaemon {
        // Under /tmp, not the default temp dir: a Unix socket path must fit in
        // SUN_LEN (108 bytes) and a session TMPDIR can be longer than that.
        let dir = tempfile::Builder::new()
            .prefix("wmx64.")
            .tempdir_in("/tmp")
            .expect("tempdir");
        let socket = dir.path().join("wamux.sock");
        let db = format!("sqlite://{}?mode=rwc", dir.path().join("w.db").display());
        let engine = Arc::new(SqlStore::open_sqlite(&db).await.expect("open sqlite"));
        let registry = Arc::new(AccountRegistry::new(engine, RegistryTuning::with_ring(64)));
        let socket_str = socket.to_str().expect("utf-8 path").to_string();
        let config = Config {
            socket_path: socket_str.clone(),
            enable_reflection: false,
            ..Config::default()
        };
        let stream = transport::uds_listener::bind(&socket_str, 0o660, None).expect("bind");
        let router = server::build_router(
            registry.clone(),
            &config,
            transport::shutdown::Shutdown::new(),
        );
        tokio::spawn(async move {
            let _ = router.serve_with_incoming(stream).await;
        });
        TestDaemon {
            socket,
            registry,
            _dir: dir,
        }
    }

    pub fn socket_str(&self) -> String {
        self.socket.to_str().expect("utf-8 path").to_string()
    }
}
