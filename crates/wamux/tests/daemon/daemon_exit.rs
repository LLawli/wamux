//! #239: the daemon builds its runtime by hand and bounds the wait for blocking
//! tasks on the way out. This drives the real binary through a clean SIGTERM,
//! so the hand-built runtime still serves, still exits 0, reports a drained
//! blocking pool, and does not report abandoned work when there was none. The
//! stalled case needs a stalled HTTPS read the binary cannot be pointed at; the
//! unit tests in `daemon_runtime_tests.rs` and the live reproduction cover it.
//! SQLite, no account, no network.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const STARTUP: Duration = Duration::from_secs(60);
/// No account and no blocking work: well past the 3 s account stop, well short
/// of a hang.
const EXIT_DEADLINE: Duration = Duration::from_secs(10);

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
    fn start(dir: &Path, socket: &Path) -> Daemon {
        let log = dir.join("daemon.log");
        let out = File::create(&log).unwrap();
        let err = out.try_clone().unwrap();
        let dsn = format!("sqlite://{}", dir.join("wamux.db").display());
        let child = Command::new(env!("CARGO_BIN_EXE_wamux"))
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

    /// SIGTERM through `kill(1)`: `unsafe_code` is forbidden, and nix's
    /// `signal` feature is not enabled for the tests.
    fn sigterm(&self) {
        let status = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .expect("run kill");
        assert!(status.success(), "kill -TERM failed: {status}");
    }

    /// The exit status, or `None` if the daemon is still running at `deadline`.
    fn wait_exit(&mut self, deadline: Duration) -> Option<ExitStatus> {
        let until = Instant::now() + deadline;
        while Instant::now() < until {
            if let Some(status) = self.child.try_wait().expect("poll the daemon") {
                return Some(status);
            }
            // not a sync point: an external process's exit is polled with a deadline; there is nothing to await
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }
}

#[test]
fn sigterm_exits_zero_and_removes_the_socket() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("wamux.sock");
    let mut daemon = Daemon::start(dir.path(), &socket);

    daemon.sigterm();
    let status = daemon.wait_exit(EXIT_DEADLINE);
    let log = daemon.log_text();

    let status = status.unwrap_or_else(|| {
        panic!("the daemon was still running {EXIT_DEADLINE:?} after SIGTERM:\n{log}")
    });
    assert!(status.success(), "expected exit 0, got {status}:\n{log}");
    assert!(!socket.exists(), "the socket file was left behind:\n{log}");
    assert!(
        log.contains("wamux stopped"),
        "no clean stop in the log:\n{log}"
    );
    // Only `block_on_bounded` knows how the blocking pool ended, so this line
    // proves the daemon exits through the bounded runtime.
    assert!(
        log.contains("blocking pool drained"),
        "the exit must report how the blocking pool ended:\n{log}"
    );
    assert!(
        !log.contains("blocking tasks still running"),
        "nothing was blocking, so nothing may be reported abandoned:\n{log}"
    );
}
