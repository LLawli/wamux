//! #218: the daemon sets its own umask. The `scratch` image has no shell, so
//! the `sh -c 'umask 077 && exec'` entrypoint of #75 is gone, and whatever the
//! daemon creates has to be owner-only by the daemon's own doing. Starts the
//! real binary under a umask of the test's choosing (`unsafe_code` is
//! forbidden, so a test cannot set its own) and reads the result from `/proc`
//! and the filesystem. No account, no WhatsApp, no network.

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
    /// The daemon on SQLite in `dir`, started under `umask` (octal text),
    /// serving at `socket`. `exec` keeps the child's pid the daemon's.
    fn start(dir: &Path, umask: &str, socket: &Path) -> Daemon {
        let log = dir.join("daemon.log");
        let out = File::create(&log).unwrap();
        let err = out.try_clone().unwrap();
        let dsn = format!("sqlite://{}", dir.join("wamux.db").display());
        let child = Command::new("sh")
            .args(["-c", &format!("umask {umask} && exec \"$0\"")])
            .arg(env!("CARGO_BIN_EXE_wamux"))
            .current_dir(dir)
            .env("WAMUX_DATABASE_URL", dsn)
            .env("WAMUX_SOCKET_PATH", socket)
            // The default spool, /var/cache/wamux, is the host's (#79).
            .env("WAMUX_MEDIA_SPOOL_DIR", dir.join("spool"))
            .env("WAMUX_LOG_LEVEL", "info,wamux=info")
            .env("NO_COLOR", "1")
            .stdout(Stdio::from(out))
            .stderr(Stdio::from(err))
            .spawn()
            .expect("spawn the daemon");
        let mut daemon = Daemon { child, log };
        daemon.wait_for_socket(socket);
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

    /// The `Umask:` line of the running daemon, as the kernel reports it.
    fn umask_line(&self) -> String {
        let status = fs::read_to_string(format!("/proc/{}/status", self.child.id()))
            .expect("read the daemon's /proc status");
        status
            .lines()
            .find(|line| line.starts_with("Umask:"))
            .unwrap_or_else(|| panic!("no Umask line in /proc status:\n{status}"))
            .split_whitespace()
            .nth(1)
            .expect("Umask value")
            .to_string()
    }
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path)
        .unwrap_or_else(|e| panic!("{path:?}: {e}"))
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn the_daemon_runs_with_umask_077_when_started_under_umask_000() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("wamux.sock");
    let daemon = Daemon::start(dir.path(), "000", &socket);

    assert_eq!(
        daemon.umask_line(),
        "0077",
        "the daemon must narrow its own umask whatever it was started with:\n{}",
        daemon.log_text()
    );
}

/// Started under 077 (what the daemon now always runs with, and what systemd's
/// `UMask=0077` already gave it): a parent level the daemon creates must still
/// be 0750, so the `socket_group` consumer can reach the socket (#77).
#[test]
fn socket_parent_levels_the_daemon_creates_are_0750_under_its_own_umask() {
    let dir = tempfile::tempdir().unwrap();
    let outer = dir.path().join("run");
    let inner = outer.join("wamux");
    let socket = inner.join("wamux.sock");
    let daemon = Daemon::start(dir.path(), "077", &socket);

    assert_eq!(
        (
            format!("{:o}", mode_of(&outer)),
            format!("{:o}", mode_of(&inner))
        ),
        ("750".to_string(), "750".to_string()),
        "both created levels must be 0750, not 0750 less the umask:\n{}",
        daemon.log_text()
    );
}
