//! The store file is a secret (#75): it holds every account's identity key,
//! Signal sessions, prekeys and sender keys in plaintext, so whoever reads it
//! can impersonate the devices. A file-backed engine (`sqlite://`, `turso://`)
//! creates it here, owner-only, regardless of the process umask, and warns when
//! a file it did not create is open to group or others.
//!
//! The warning is the whole policy for an existing file: the core does not
//! chmod an operator's file or refuse to start (the operator may share it with
//! a backup user on purpose), it makes the exposure visible. SQLite creates
//! `-wal` and `-shm` with the mode of the main file (measured under umask 022),
//! so creating the main file 0600 covers all three.

use std::io;
use std::path::{Path, PathBuf};

use wacore::store::error::Result as StoreResult;

/// Create `path` empty with mode 0600 if it does not exist; an existing file is
/// left exactly as it is. `create_new` makes this atomic: there is no window in
/// which the file exists with a looser mode. A no-op off Unix.
#[cfg(unix)]
pub(crate) fn create_private(path: &Path) -> io::Result<()> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;

    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
    {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(not(unix))]
pub(crate) fn create_private(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// The main file, `<path>-wal` and `<path>-shm` that exist and are readable or
/// writable by group or others (`mode & 0o077 != 0`), with their modes.
#[cfg(unix)]
pub(crate) fn loose_files(path: &Path) -> Vec<(PathBuf, u32)> {
    use std::os::unix::fs::PermissionsExt;

    ["", "-wal", "-shm"]
        .iter()
        .map(|suffix| with_suffix(path, suffix))
        .filter_map(|file| {
            let mode = std::fs::metadata(&file).ok()?.permissions().mode() & 0o777;
            (mode & 0o077 != 0).then_some((file, mode))
        })
        .collect()
}

#[cfg(not(unix))]
pub(crate) fn loose_files(_path: &Path) -> Vec<(PathBuf, u32)> {
    Vec::new()
}

/// `path` with `suffix` appended to the file name (SQLite's `-wal`/`-shm`).
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// One `warn!` per loose file: the path, its octal mode and the `chmod 600`
/// that fixes it.
pub(crate) fn warn_if_loose(path: &Path) {
    for (file, mode) in loose_files(path) {
        tracing::warn!(
            path = %file.display(),
            mode = format!("{mode:o}"),
            "store file is readable by group or others and holds every account's keys (#75); run: chmod 600 {}",
            file.display()
        );
    }
}

/// `create_private` then `warn_if_loose`: what an engine calls before it opens
/// the file. An I/O failure is a `StoreError::Io`.
pub(crate) fn secure_store_file(path: &Path) -> StoreResult<()> {
    create_private(path)?;
    warn_if_loose(path);
    Ok(())
}

#[cfg(test)]
#[path = "file_mode_tests.rs"]
mod tests;
