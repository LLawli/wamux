//! #79: the spool directory is created 0700 at startup (only the levels that
//! are missing) and must be writable, or startup stops naming it.

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

use super::{SpoolDirError, prepare_spool_dir};

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).expect("metadata").mode() & 0o7777
}

/// The process umask, read without changing it (Linux 4.7+).
fn current_umask() -> u32 {
    let status = std::fs::read_to_string("/proc/self/status").expect("/proc/self/status");
    let line = status
        .lines()
        .find_map(|line| line.strip_prefix("Umask:"))
        .expect("an Umask: line");
    u32::from_str_radix(line.trim(), 8).expect("octal umask")
}

#[test]
fn prepare_creates_missing_levels_0700() {
    let root = tempfile::tempdir().expect("tempdir");
    let cache = root.path().join("cache");
    let spool = cache.join("wamux");
    prepare_spool_dir(&spool).expect("prepare");
    let expected = 0o700 & !current_umask();
    assert_eq!(mode(&cache), expected, "a created level");
    assert_eq!(mode(&spool), expected, "the spool itself");
    assert_eq!(
        std::fs::read_dir(&spool).expect("read_dir").count(),
        0,
        "the writability probe leaves nothing behind"
    );
}

#[test]
fn prepare_keeps_an_existing_dir_mode() {
    let root = tempfile::tempdir().expect("tempdir");
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o750)).expect("chmod");
    prepare_spool_dir(root.path()).expect("prepare");
    assert_eq!(mode(root.path()), 0o750);
}

#[test]
fn prepare_refuses_a_read_only_dir_naming_it() {
    assert!(
        !nix::unistd::geteuid().is_root(),
        "run as a regular user: root writes anywhere"
    );
    let root = tempfile::tempdir().expect("tempdir");
    let spool = root.path().join("spool");
    std::fs::create_dir(&spool).expect("mkdir");
    std::fs::set_permissions(&spool, std::fs::Permissions::from_mode(0o500)).expect("chmod");
    let err = prepare_spool_dir(&spool).expect_err("a read-only spool");
    let shown = spool.display().to_string();
    assert!(
        matches!(&err, SpoolDirError::NotWritable { path, .. } if *path == shown),
        "{err:?}"
    );
    assert!(err.to_string().contains(&shown), "{err}");
    std::fs::set_permissions(&spool, std::fs::Permissions::from_mode(0o700)).expect("chmod back");
}
