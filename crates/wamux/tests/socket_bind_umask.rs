//! #77: the socket never appears at its path with a mode wider than
//! `socket_mode`, not even under umask 0000. Before the fix the daemon bound
//! (the socket came out 0777 minus the umask) and chmodded afterwards, so for a
//! moment any local user could connect.
//!
//! A test binary of its own: the umask belongs to the process, and no other
//! test may run while it is 0000.

use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nix::sys::stat::{Mode, umask};
use wamux::transport::uds_listener;

/// Each round is one bind with a poller spinning on the path; the window it
/// hunts is a few microseconds wide, so it takes many rounds to be sure.
const ROUNDS: usize = 300;
const SOCKET_MODE: u32 = 0o660;

/// Every permission bit the path showed while `bind` ran, plus the final one.
fn modes_seen_during(path: PathBuf, bind: impl FnOnce()) -> u32 {
    let spinning = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let poller = {
        let (spinning, stop) = (spinning.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut seen = 0;
            spinning.store(true, Ordering::Release);
            loop {
                let last = stop.load(Ordering::Acquire);
                if let Ok(meta) = std::fs::symlink_metadata(&path) {
                    seen |= meta.mode() & 0o777;
                }
                if last {
                    return seen;
                }
            }
        })
    };
    while !spinning.load(Ordering::Acquire) {
        std::hint::spin_loop();
    }
    bind();
    stop.store(true, Ordering::Release);
    poller.join().expect("poller thread")
}

#[tokio::test]
async fn the_socket_never_appears_wider_than_socket_mode_under_umask_0000() {
    umask(Mode::empty());
    for round in 0..ROUNDS {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("wamux.sock");
        let path_str = path.to_str().expect("utf-8 path").to_string();
        let mut listener = None;
        let seen = modes_seen_during(path, || {
            listener = Some(uds_listener::bind(&path_str, SOCKET_MODE, None).expect("bind"));
        });
        assert_eq!(
            seen, SOCKET_MODE,
            "round {round}: the socket showed mode {seen:o} at its path, expected only {SOCKET_MODE:o}"
        );
    }
}
