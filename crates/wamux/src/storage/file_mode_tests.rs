//! #75: a new store file is owner-only, an existing one is never touched, and
//! the loose ones are found. Unix modes only.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use super::{create_private, loose_files};

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn write_with_mode(path: &Path, mode: u32) {
    fs::write(path, b"").unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn create_private_makes_a_new_file_0600() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wamux.db");
    create_private(&path).unwrap();
    assert!(path.is_file());
    assert_eq!(mode_of(&path), 0o600);
}

#[test]
fn create_private_leaves_an_existing_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wamux.db");
    fs::write(&path, b"keep me").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    create_private(&path).unwrap();
    assert_eq!(mode_of(&path), 0o644, "the mode is the operator's");
    assert_eq!(fs::read(&path).unwrap(), b"keep me", "the content too");
}

#[test]
fn create_private_fails_when_the_directory_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing").join("wamux.db");
    let error = create_private(&path).expect_err("no parent directory");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn loose_files_lists_the_main_file_and_the_wal_and_shm_that_exist() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wamux.db");
    write_with_mode(&path, 0o644);
    write_with_mode(&dir.path().join("wamux.db-wal"), 0o640);
    write_with_mode(&dir.path().join("wamux.db-shm"), 0o600);
    let mut loose = loose_files(&path);
    loose.sort();
    assert_eq!(
        loose,
        vec![
            (path.clone(), 0o644),
            (dir.path().join("wamux.db-wal"), 0o640),
        ],
        "the 0600 -shm is fine"
    );
}

#[test]
fn loose_files_is_empty_for_0600_and_for_a_file_that_does_not_exist() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wamux.db");
    assert!(loose_files(&path).is_empty(), "nothing there yet");
    write_with_mode(&path, 0o600);
    assert!(loose_files(&path).is_empty());
}
