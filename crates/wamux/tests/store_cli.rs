//! #165: `wamux store decrypt --yes`, through the real binary. The library tests
//! (`store_conversion.rs`) cover the decryption itself; these cover what an
//! operator meets: the confirmation, the key file, the exit codes and that
//! `wamux` with no arguments still serves.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use wamux::storage::StorageEngine;
use wamux::storage::sql::SqlStore;

#[allow(dead_code)]
mod common;

use common::store_secrets::*;

const WAMUX: &str = env!("CARGO_BIN_EXE_wamux");

/// Long enough for a migration and a decrypt, short enough to fail a build that
/// ignores its arguments and serves forever.
const PATIENCE: Duration = Duration::from_secs(60);

trait RunBounded {
    /// Run to the end, or kill it after `PATIENCE` and fail the test.
    fn run(&mut self) -> Output;
}

impl RunBounded for Command {
    fn run(&mut self) -> Output {
        let mut child = self
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn wamux");
        let deadline = Instant::now() + PATIENCE;
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                let _ = child.kill();
                let output = child.wait_with_output().unwrap();
                panic!("wamux did not finish in {PATIENCE:?}:\n{}", text(&output));
            }
            // not a sync point: waiting for an external process to exit, polled with a deadline; there is no event to await
            std::thread::sleep(Duration::from_millis(50));
        }
        child.wait_with_output().unwrap()
    }
}

struct Setup {
    dir: tempfile::TempDir,
}

impl Setup {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn url(&self) -> String {
        sqlite_url(self.dir.path())
    }

    fn key_file(&self) -> PathBuf {
        let path = self.dir.path().join("store-key");
        fs::write(&path, format!("{}\n", KEY_A.repeat(32))).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        path
    }

    /// The daemon binary with this store's config in its environment.
    fn wamux(&self, args: &[&str], key_file: Option<&Path>) -> Command {
        let mut command = Command::new(WAMUX);
        command
            .args(args)
            .current_dir(self.dir.path())
            .env("WAMUX_DATABASE_URL", self.url())
            .env("WAMUX_SOCKET_PATH", self.dir.path().join("wamux.sock"))
            .env("NO_COLOR", "1");
        if let Some(path) = key_file {
            command.env("WAMUX_STORE_KEY_FILE", path);
        }
        command
    }

    /// An encrypted SQLite store with two accounts, closed.
    async fn encrypted_store(&self) {
        let store = SqlStore::open_sqlite_keyed(&self.url(), Some(&key(KEY_A)))
            .await
            .unwrap();
        for name in ["cli/a", "cli/b"] {
            let account = store.create_account(Some(name)).await.unwrap();
            write_every_secret(&*store.device_backend(account.device_id)).await;
        }
        common::lite_pool(&store).close().await;
    }

    async fn state(&self) -> String {
        let store = SqlStore::open_sqlite_keyed(&self.url(), Some(&key(KEY_A)))
            .await
            .ok();
        match store {
            Some(store) => store.encryption_state().await,
            None => {
                let plain = SqlStore::open_sqlite(&self.url()).await.unwrap();
                plain.encryption_state().await
            }
        }
    }
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[tokio::test]
async fn store_decrypt_without_yes_refuses_and_changes_nothing() {
    let setup = Setup::new();
    setup.encrypted_store().await;
    let key_file = setup.key_file();
    let output = setup.wamux(&["store", "decrypt"], Some(&key_file)).run();
    assert!(!output.status.success(), "{}", text(&output));
    let shown = text(&output);
    assert!(shown.contains("--yes"), "{shown}");
    assert_eq!(setup.state().await, "encrypted");
}

#[tokio::test]
async fn store_decrypt_without_a_key_file_refuses() {
    let setup = Setup::new();
    setup.encrypted_store().await;
    let output = setup.wamux(&["store", "decrypt", "--yes"], None).run();
    assert!(!output.status.success(), "{}", text(&output));
    // The daemon's own refusal also names store_key_file; this one is the
    // command's, so it must name the command too.
    let shown = text(&output);
    assert!(shown.contains("store_key_file"), "{shown}");
    assert!(shown.contains("decrypt"), "{shown}");
    assert_eq!(setup.state().await, "encrypted");
}

#[tokio::test]
async fn store_decrypt_decrypts_an_encrypted_sqlite_store() {
    let setup = Setup::new();
    setup.encrypted_store().await;
    let key_file = setup.key_file();
    let output = setup
        .wamux(&["store", "decrypt", "--yes"], Some(&key_file))
        .run();
    assert!(output.status.success(), "{}", text(&output));
    assert!(
        text(&output).contains("decrypted 2 account"),
        "{}",
        text(&output)
    );

    let store = SqlStore::open_sqlite(&setup.url())
        .await
        .expect("opens with no key");
    assert_eq!(store.encryption_state().await, "plaintext");
    assert_eq!(
        store.blobs("sessions", "record").await,
        vec![MARKER.to_vec(), MARKER.to_vec()]
    );
}

#[tokio::test]
async fn store_decrypt_refuses_a_plaintext_store() {
    let setup = Setup::new();
    let store = SqlStore::open_sqlite(&setup.url()).await.unwrap();
    common::lite_pool(&store).close().await;
    let key_file = setup.key_file();
    let output = setup
        .wamux(&["store", "decrypt", "--yes"], Some(&key_file))
        .run();
    assert!(!output.status.success(), "{}", text(&output));
    assert!(text(&output).contains("not encrypted"), "{}", text(&output));
}

#[test]
fn an_unknown_subcommand_exits_2() {
    let setup = Setup::new();
    let output = setup.wamux(&["bogus"], None).run();
    assert_eq!(output.status.code(), Some(2), "{}", text(&output));
    assert!(text(&output).contains("store decrypt"), "{}", text(&output));
}

/// The daemon, killed when the test ends however it ends.
struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn no_arguments_still_starts_the_daemon() {
    let setup = Setup::new();
    let socket = setup.dir.path().join("wamux.sock");
    let mut command = setup.wamux(&[], None);
    let mut daemon = Daemon(
        command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(60);
    while !socket.exists() {
        assert!(daemon.0.try_wait().unwrap().is_none(), "the daemon exited");
        assert!(Instant::now() < deadline, "no socket");
        // not a sync point: the socket appearing is the signal, polled with a deadline; an external process offers nothing to await
        std::thread::sleep(Duration::from_millis(100));
    }
}
