//! #75: the store file is a secret. Starts the real daemon, because the claim is
//! about files the daemon creates under a permissive umask, and `unsafe_code`
//! is forbidden so a test cannot set its own umask: `sh -c 'umask 022 && exec'`
//! does it for the child. No account, no WhatsApp, no network: the daemon only
//! opens its store and binds its socket.

use std::fs::{self, File};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const STARTUP: Duration = Duration::from_secs(60);

/// The daemon, killed when the test ends however it ends.
struct Daemon {
    child: Child,
    log: PathBuf,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Daemon {
    fn start(dir: &Path, dsn: &str) -> Daemon {
        let log = dir.join("daemon.log");
        let out = File::create(&log).unwrap();
        let err = out.try_clone().unwrap();
        let child = Command::new("sh")
            .args(["-c", "umask 022 && exec \"$0\""])
            .arg(env!("CARGO_BIN_EXE_wamux"))
            .current_dir(dir)
            .env("WAMUX_DATABASE_URL", dsn)
            .env("WAMUX_SOCKET_PATH", dir.join("wamux.sock"))
            .env("WAMUX_LOG_LEVEL", "info,wamux=info")
            .env("NO_COLOR", "1")
            .stdout(Stdio::from(out))
            .stderr(Stdio::from(err))
            .spawn()
            .expect("spawn the daemon");
        let mut daemon = Daemon { child, log };
        daemon.wait_for_socket(&dir.join("wamux.sock"));
        daemon
    }

    fn wait_for_socket(&mut self, socket: &Path) {
        let deadline = Instant::now() + STARTUP;
        while !socket.exists() {
            if let Ok(Some(status)) = self.child.try_wait() {
                panic!("the daemon exited early ({status}):\n{}", self.log_text());
            }
            assert!(Instant::now() < deadline, "no socket:\n{}", self.log_text());
            // not a sync point: the socket appearing is the signal, polled with a deadline; an external process offers nothing to await
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn log_text(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path)
        .unwrap_or_else(|e| panic!("{path:?}: {e}"))
        .permissions()
        .mode()
        & 0o777
}

fn sibling(db: &Path, suffix: &str) -> PathBuf {
    let mut name = db.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn plant_loose_file(db: &Path) {
    fs::write(db, b"").unwrap();
    fs::set_permissions(db, fs::Permissions::from_mode(0o644)).unwrap();
}

#[test]
fn sqlite_store_is_created_0600_with_wal_and_shm_under_umask_022() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("wamux.db");
    let _daemon = Daemon::start(dir.path(), &format!("sqlite://{}", db.display()));
    for file in [db.clone(), sibling(&db, "-wal"), sibling(&db, "-shm")] {
        assert_eq!(mode_of(&file), 0o600, "{file:?}");
    }
}

#[test]
fn sqlite_store_warns_about_an_existing_0644_file_and_leaves_it_alone() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("wamux.db");
    plant_loose_file(&db);
    let daemon = Daemon::start(dir.path(), &format!("sqlite://{}", db.display()));
    let log = daemon.log_text();
    assert!(log.contains("WARN"), "{log}");
    assert!(log.contains(db.to_str().unwrap()), "{log}");
    assert!(log.contains("644"), "{log}");
    assert_eq!(mode_of(&db), 0o644, "the daemon does not chmod it");
}

#[cfg(feature = "turso")]
#[test]
fn turso_store_is_created_0600_under_umask_022() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("wamux.db");
    let _daemon = Daemon::start(dir.path(), &format!("turso://{}", db.display()));
    assert_eq!(mode_of(&db), 0o600);
}

#[cfg(feature = "turso")]
#[test]
fn turso_store_warns_about_an_existing_0644_file() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("wamux.db");
    plant_loose_file(&db);
    let daemon = Daemon::start(dir.path(), &format!("turso://{}", db.display()));
    let log = daemon.log_text();
    assert!(log.contains("WARN"), "{log}");
    assert!(log.contains(db.to_str().unwrap()), "{log}");
    assert_eq!(mode_of(&db), 0o644);
}
