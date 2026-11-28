//! #277: opening a `sqlite://` store whose directory does not exist creates the
//! directory, mode 0700, the way the media spool already creates its own. The
//! Android default (`$PREFIX/var/lib/wamux/wamux.db`) names a directory a fresh
//! Termux does not have, and SQLite itself creates only the file, so without
//! this the daemon stops at "opening storage: No such file or directory". 0700
//! because the store holds every account's identity keys. A directory that is
//! already there is the operator's, and its mode is left alone.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path).expect("stat").permissions().mode() & 0o777
}

#[tokio::test]
async fn sqlite_open_creates_a_missing_store_directory_0700() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("var").join("lib").join("wamux");
    let file = dir.join("wamux.db");
    let dsn = format!("sqlite://{}", file.display());

    let engine = wamux::storage::open_engine(&dsn, 4).await;

    assert!(engine.is_ok(), "open {dsn}: {:?}", engine.err());
    assert!(
        file.is_file(),
        "the store file exists at {}",
        file.display()
    );
    assert_eq!(mode_of(&dir), 0o700, "{}", dir.display());
}

#[tokio::test]
async fn sqlite_open_leaves_an_existing_directory_mode_alone() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("store");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o750)).unwrap();
    let file = dir.join("wamux.db");
    let dsn = format!("sqlite://{}", file.display());

    let engine = wamux::storage::open_engine(&dsn, 4).await;

    assert!(engine.is_ok(), "open {dsn}: {:?}", engine.err());
    assert!(file.is_file());
    assert_eq!(
        mode_of(&dir),
        0o750,
        "the operator's directory keeps its mode"
    );
}
