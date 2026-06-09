//! Bind the Unix domain socket: unlink any stale file, bind, then chmod to the
//! configured mode before announcing readiness. The owning group is taken from
//! the daemon's process group (run wamux as that group); chown is not performed
//! here to avoid a libc dependency and a CAP_CHOWN requirement.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;

pub fn bind(path: &str, mode: u32, group: Option<&str>) -> anyhow::Result<UnixListenerStream> {
    let socket = Path::new(path);
    if socket.exists() {
        // Stale socket from a previous run; safe to remove before re-binding.
        std::fs::remove_file(socket)?;
    }
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(mode))?;

    if let Some(group) = group {
        // The socket inherits the daemon's gid; document the run-as-group requirement.
        tracing::info!(
            group,
            "socket group is the daemon's process group (no chown performed)"
        );
    }

    Ok(UnixListenerStream::new(listener))
}

/// Best-effort unlink on shutdown.
pub fn unlink(path: &str) {
    let _ = std::fs::remove_file(path);
}
