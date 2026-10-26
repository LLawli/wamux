//! Bind the Unix domain socket (#77). A file already at the path is probed
//! before anything is removed: a live socket (another wamux) or something that
//! is not a socket stops startup, and only a socket that refuses connections is
//! treated as stale. The socket is bound in a private directory, given its
//! group and mode there, and renamed onto the path, so it never appears at the
//! path with a wider mode or the wrong group.

use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};

use nix::unistd::{Gid, Group};
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;

/// Why the socket could not be bound. Each variant names the path or group to
/// fix; an `io::Error` underneath rides as the source.
#[derive(Debug, thiserror::Error)]
pub enum BindError {
    #[error("{path} exists and is not a socket; refusing to remove it")]
    NotASocket { path: String },

    #[error("another wamux is serving {path}")]
    InUse { path: String },

    /// Neither a connection nor a refusal: the socket cannot be proven stale,
    /// so it is left alone.
    #[error("cannot tell whether {path} is in use")]
    Probe {
        path: String,
        #[source]
        source: io::Error,
    },

    #[error("socket_group {group:?} does not exist")]
    GroupNotFound { group: String },

    #[error("the daemon's user is not a member of group {group:?}, so it cannot give {path} to it")]
    NotAMember { group: String, path: String },

    /// Any other filesystem step; `step` says which (`"bind"`, `"chmod"`, ...).
    #[error("{step} {path}")]
    Io {
        step: &'static str,
        path: String,
        #[source]
        source: io::Error,
    },
}

/// What the probe found at the path.
enum Probed {
    /// Nothing there.
    Absent,
    /// A socket that refuses connections: nobody is serving it.
    Stale,
}

/// Mode of the private bind directory: it is only a staging area (#77).
const PRIVATE_DIR_MODE: u32 = 0o700;
/// Mode of any parent level the daemon creates (set by chmod, so the daemon's
/// umask does not narrow it); an existing one is not touched.
const CREATED_PARENT_MODE: u32 = 0o750;

/// Bind `path` with `mode` and, when set, `group` (a name, or a number taken as
/// a gid).
///
/// The socket is bound in `<parent>/.wamux-bind-<pid>/s`, given its group and
/// mode there, and renamed onto `path`, so the path only ever shows the final
/// mode and group (#77; bind-then-chmod would leave a window at 0777 minus umask).
/// The group is resolved first so a bad name creates nothing. A listening
/// socket keeps accepting across the rename.
pub fn bind(path: &str, mode: u32, group: Option<&str>) -> Result<UnixListenerStream, BindError> {
    let gid = group.map(resolve_group).transpose()?;
    if let Probed::Stale = probe(path)? {
        std::fs::remove_file(path)
            .map_err(|source| io_error("remove stale socket", path, source))?;
    }
    let parent = parent_of(path);
    create_parent(&parent, path)?;
    let staging = parent.join(format!(".wamux-bind-{}", std::process::id()));
    let result = bind_in_staging(&staging, path, mode, group.zip(gid));
    // On success the directory is empty, on failure it may hold the socket.
    let _ = std::fs::remove_dir_all(&staging);
    result
}

/// The probe `bind` runs, on its own: `main` calls it before opening the store,
/// so a second daemon stops before it runs migrations on the live one's store
/// (#77, #173: a newer binary must never migrate a store an older daemon is still
/// serving). Absent or stale (refuses connections) is `Ok`; nothing is removed.
pub fn refuse_if_served(path: &str) -> Result<(), BindError> {
    probe(path).map(|_| ())
}

/// Look at `path` without changing anything. Only a socket that refuses
/// connections counts as stale; anything unproven is left alone.
fn probe(path: &str) -> Result<Probed, BindError> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(Probed::Absent),
        Err(source) => {
            return Err(BindError::Probe {
                path: path.to_string(),
                source,
            });
        }
    };
    if !meta.file_type().is_socket() {
        return Err(BindError::NotASocket {
            path: path.to_string(),
        });
    }
    match std::os::unix::net::UnixStream::connect(path) {
        Ok(_) => Err(BindError::InUse {
            path: path.to_string(),
        }),
        Err(source) if source.kind() == io::ErrorKind::ConnectionRefused => Ok(Probed::Stale),
        Err(source) => Err(BindError::Probe {
            path: path.to_string(),
            source,
        }),
    }
}

/// A numeric string is a gid as-is; a name goes through NSS (`/etc/group`,
/// systemd-userdb, LDAP), which is why this is `nix` and not a file parse.
fn resolve_group(group: &str) -> Result<Gid, BindError> {
    if let Ok(number) = group.parse::<u32>() {
        return Ok(Gid::from_raw(number));
    }
    match Group::from_name(group) {
        Ok(Some(found)) => Ok(found.gid),
        Ok(None) => Err(BindError::GroupNotFound {
            group: group.to_string(),
        }),
        Err(errno) => Err(io_error("look up group", group, errno.into())),
    }
}

fn io_error(step: &'static str, path: &str, source: io::Error) -> BindError {
    BindError::Io {
        step,
        path: path.to_string(),
        source,
    }
}

/// The directory the socket lives in; a bare file name means the cwd.
fn parent_of(path: &str) -> PathBuf {
    match Path::new(path).parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Create the missing levels and leave each one 0750, whatever the umask; a
/// parent that exists is not touched.
///
/// `DirBuilder::mode` is reduced by the umask, and the daemon runs under 077:
/// that would make a created level 0700, and the `socket_group` consumer could
/// not traverse it to reach the socket. So the levels are chmodded explicitly.
fn create_parent(parent: &Path, path: &str) -> Result<(), BindError> {
    for level in missing_levels(parent) {
        match create_level(&level) {
            Ok(()) => {}
            // Made by someone else since the check: like any existing parent,
            // it keeps the mode its creator gave it.
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {}
            Err(source) => return Err(io_error("create parent of", path, source)),
        }
    }
    Ok(())
}

/// One new level at `CREATED_PARENT_MODE`: created with it, then chmodded
/// because the creation mode is reduced by the umask.
fn create_level(level: &Path) -> io::Result<()> {
    std::fs::DirBuilder::new()
        .mode(CREATED_PARENT_MODE)
        .create(level)?;
    std::fs::set_permissions(level, std::fs::Permissions::from_mode(CREATED_PARENT_MODE))
}

/// The levels of `parent` that do not exist yet, outermost first.
fn missing_levels(parent: &Path) -> Vec<PathBuf> {
    let mut levels: Vec<PathBuf> = parent
        .ancestors()
        .take_while(|level| !level.as_os_str().is_empty() && !level.exists())
        .map(Path::to_path_buf)
        .collect();
    levels.reverse();
    levels
}

/// Bind in `staging`, finish group and mode there, then rename onto `path`.
/// `group` is the configured name with its resolved gid.
fn bind_in_staging(
    staging: &Path,
    path: &str,
    mode: u32,
    group: Option<(&str, Gid)>,
) -> Result<UnixListenerStream, BindError> {
    // A leftover from a crashed run with the same pid would make the bind fail.
    let _ = std::fs::remove_dir_all(staging);
    std::fs::DirBuilder::new()
        .mode(PRIVATE_DIR_MODE)
        .create(staging)
        .map_err(|source| io_error("create private bind directory for", path, source))?;
    let staged = staging.join("s");
    let listener = UnixListener::bind(&staged).map_err(|source| io_error("bind", path, source))?;
    if let Some((name, gid)) = group {
        give_to_group(&staged, gid, name, path)?;
    }
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(mode))
        .map_err(|source| io_error("chmod", path, source))?;
    std::fs::rename(&staged, path).map_err(|source| io_error("rename onto", path, source))?;
    tracing::info!(
        path,
        mode = format_args!("{mode:o}"),
        group = group.map(|(name, _)| name),
        "unix socket bound"
    );
    Ok(UnixListenerStream::new(listener))
}

/// `EPERM` means the daemon's user is not in the group (root excepted).
fn give_to_group(staged: &Path, gid: Gid, name: &str, path: &str) -> Result<(), BindError> {
    match std::os::unix::fs::chown(staged, None, Some(gid.as_raw())) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::PermissionDenied => {
            Err(BindError::NotAMember {
                group: name.to_string(),
                path: path.to_string(),
            })
        }
        Err(source) => Err(io_error("chown", path, source)),
    }
}

/// Best-effort unlink on shutdown.
pub fn unlink(path: &str) {
    let _ = std::fs::remove_file(path);
}
