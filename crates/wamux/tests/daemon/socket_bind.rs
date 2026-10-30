//! #77: binding the socket. A file already at the path is probed before
//! anything is removed, the socket gets a real `socket_group`, and a parent the
//! daemon creates is 0750. Real filesystem, one tempdir per test.
//!
//! The group tests need the user in a supplementary group (CI's runner is in
//! `docker` and `adm`) and not running as root (root may chown to any group).
//! They run on Linux only: nix has no `getgroups` on Apple targets (#231), and
//! the chown they exercise is the same code on every Unix.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tempfile::TempDir;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::UnixListenerStream;

use wamux::transport::uds_listener::{self, BindError};

const MODE: u32 = 0o660;

/// A fresh directory and the socket path inside it.
fn spot() -> (TempDir, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("wamux.sock");
    (dir, path.to_str().expect("utf-8 path").to_string())
}

/// Names in `dir`, hidden ones included, sorted.
fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("read_dir")
        .map(|entry| entry.expect("entry").file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn permissions(path: impl AsRef<Path>) -> u32 {
    std::fs::metadata(path).expect("metadata").mode() & 0o7777
}

/// The listener behind `stream` accepts a connection made to `path`.
async fn assert_accepts(stream: &mut UnixListenerStream, path: &str) {
    let _client = tokio::net::UnixStream::connect(path)
        .await
        .expect("connect");
    tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("accepted in time")
        .expect("listener open")
        .expect("accept");
}

#[tokio::test]
async fn a_live_socket_at_the_path_refuses_the_second_bind() {
    let (dir, path) = spot();
    let mut first = uds_listener::bind(&path, MODE, None).expect("first bind");
    let inode = std::fs::metadata(&path).expect("metadata").ino();

    let err = uds_listener::bind(&path, MODE, None).expect_err("a live socket is not taken over");

    assert!(
        matches!(&err, BindError::InUse { path: p } if *p == path),
        "{err:?}"
    );
    assert!(err.to_string().contains(&path), "{err}");
    let still = std::fs::metadata(&path).expect("metadata").ino();
    assert_eq!(still, inode, "the live socket file was replaced");
    assert_accepts(&mut first, &path).await;
    assert_eq!(entries(dir.path()), ["wamux.sock"]);
}

#[tokio::test]
async fn a_stale_socket_is_replaced() {
    let (_dir, path) = spot();
    drop(std::os::unix::net::UnixListener::bind(&path).expect("stale listener"));
    assert!(
        Path::new(&path).exists(),
        "the dead socket file stays behind"
    );

    let mut stream = uds_listener::bind(&path, MODE, None).expect("a dead socket is replaced");

    assert_accepts(&mut stream, &path).await;
    assert_eq!(permissions(&path), MODE);
}

#[tokio::test]
async fn a_regular_file_at_the_path_is_refused() {
    let (_dir, path) = spot();
    std::fs::write(&path, "not a socket").expect("write");

    let err = uds_listener::bind(&path, MODE, None).expect_err("a regular file is not removed");

    assert!(
        matches!(&err, BindError::NotASocket { path: p } if *p == path),
        "{err:?}"
    );
    assert!(err.to_string().contains(&path), "{err}");
    let content = std::fs::read_to_string(&path).expect("the file is still there");
    assert_eq!(content, "not a socket");
}

#[tokio::test]
async fn the_parent_directory_is_created_0750() {
    let dir = tempfile::tempdir().expect("tempdir");
    let existing = permissions(dir.path());
    let run: PathBuf = dir.path().join("run");
    let parent = run.join("wamux");
    let path = parent.join("wamux.sock");

    let _stream = uds_listener::bind(path.to_str().unwrap(), MODE, None).expect("bind");

    // Exactly 0750 whatever the umask: each created level is chmodded after
    // the mkdir (#218), since the daemon's own umask of 077 would make it 0700.
    assert_eq!(permissions(&run), 0o750, "a created level");
    assert_eq!(permissions(&parent), 0o750, "the socket's own parent");
    assert_eq!(
        permissions(dir.path()),
        existing,
        "an existing parent is not touched"
    );
}

#[tokio::test]
async fn an_unknown_group_fails_startup() {
    let (_dir, path) = spot();
    let missing = "wamux-77-no-such-group";

    let err = uds_listener::bind(&path, MODE, Some(missing)).expect_err("an unknown group fails");

    assert!(
        matches!(&err, BindError::GroupNotFound { group } if group == missing),
        "{err:?}"
    );
    assert!(err.to_string().contains(missing), "{err}");
    assert!(!Path::new(&path).exists(), "no socket is left at the path");
}

/// `main` runs this before opening the store, so a second daemon never touches
/// the live one's database. It only probes: nothing is removed.
#[tokio::test]
async fn the_early_probe_refuses_a_live_socket_or_a_file_and_removes_nothing() {
    let (_dir, live) = spot();
    let mut first = uds_listener::bind(&live, MODE, None).expect("bind");
    let err = uds_listener::refuse_if_served(&live).expect_err("a live socket");
    assert!(
        matches!(&err, BindError::InUse { path } if *path == live),
        "{err:?}"
    );
    assert_accepts(&mut first, &live).await;

    let (_dir, file) = spot();
    std::fs::write(&file, "not a socket").expect("write");
    let err = uds_listener::refuse_if_served(&file).expect_err("a regular file");
    assert!(
        matches!(&err, BindError::NotASocket { path } if *path == file),
        "{err:?}"
    );

    let (_dir, stale) = spot();
    drop(std::os::unix::net::UnixListener::bind(&stale).expect("stale listener"));
    uds_listener::refuse_if_served(&stale).expect("a dead socket is free to take");
    assert!(Path::new(&stale).exists(), "the probe removes nothing");

    let (_dir, absent) = spot();
    uds_listener::refuse_if_served(&absent).expect("an absent path is free");
}

#[cfg(target_os = "linux")]
mod groups {
    use nix::unistd::{Gid, Group, getegid, geteuid, getgroups};

    use super::*;

    /// A group the user is in that is not its primary one.
    fn supplementary_group() -> Group {
        let primary = getegid();
        let gid = getgroups()
            .expect("getgroups")
            .into_iter()
            .find(|gid| *gid != primary)
            .expect("this test needs the user in a supplementary group");
        Group::from_gid(gid)
            .expect("group lookup")
            .expect("the supplementary group has a name")
    }

    /// gid 0's group, which a regular user is not in.
    fn a_group_we_are_not_in() -> Group {
        assert!(!geteuid().is_root(), "run as a regular user");
        let root = Gid::from_raw(0);
        let ours = getgroups().expect("getgroups");
        assert!(
            getegid() != root && !ours.contains(&root),
            "the user is in gid 0"
        );
        Group::from_gid(root)
            .expect("group lookup")
            .expect("gid 0 has a name")
    }

    #[tokio::test]
    async fn socket_group_sets_a_supplementary_group_by_name() {
        let (_dir, path) = spot();
        let group = supplementary_group();

        let _stream = uds_listener::bind(&path, MODE, Some(&group.name)).expect("bind");

        let meta = std::fs::metadata(&path).expect("metadata");
        assert_eq!(meta.gid(), group.gid.as_raw(), "group {}", group.name);
        assert_eq!(meta.mode() & 0o7777, MODE);
    }

    #[tokio::test]
    async fn socket_group_by_number_sets_the_group() {
        let (_dir, path) = spot();
        let group = supplementary_group();
        let number = group.gid.as_raw().to_string();

        let _stream = uds_listener::bind(&path, MODE, Some(&number)).expect("bind");

        let meta = std::fs::metadata(&path).expect("metadata");
        assert_eq!(meta.gid(), group.gid.as_raw(), "gid {number}");
        assert_eq!(meta.mode() & 0o7777, MODE);
    }

    #[tokio::test]
    async fn a_group_the_daemon_is_not_in_fails_startup() {
        let (_dir, path) = spot();
        let group = a_group_we_are_not_in();

        let err = uds_listener::bind(&path, MODE, Some(&group.name))
            .expect_err("chown to a group we are not in must fail");

        assert!(
            matches!(&err, BindError::NotAMember { group: g, path: p } if *g == group.name && *p == path),
            "{err:?}"
        );
        assert!(err.to_string().contains(&group.name), "{err}");
        assert!(!Path::new(&path).exists(), "no socket is left at the path");
    }

    #[tokio::test]
    async fn no_private_bind_directory_is_left_behind() {
        let (dir, path) = spot();
        let _stream = uds_listener::bind(&path, MODE, None).expect("bind");
        assert_eq!(entries(dir.path()), ["wamux.sock"], "after a bind");

        let (failed_dir, failed_path) = spot();
        let group = a_group_we_are_not_in();
        uds_listener::bind(&failed_path, MODE, Some(&group.name))
            .expect_err("chown to a group we are not in must fail");
        assert!(
            entries(failed_dir.path()).is_empty(),
            "after a failed chown: {:?}",
            entries(failed_dir.path())
        );
    }
}
