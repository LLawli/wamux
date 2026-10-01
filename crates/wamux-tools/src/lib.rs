//! Shared client code for the wamux live and validation binaries (#64).
//!
//! Every binary in `src/bin/` reaches the daemon, reads its configuration and
//! reports its checks through this crate, so a fix to any of those lands once.
//! The env contract every binary follows lives in `live_env`; the runbook is
//! `crates/wamux-tools/README.md`.

pub mod delivery;
pub mod inproc;
pub mod live_env;
pub mod media_kit;
pub mod profile;
pub mod qr;
pub mod report;
pub mod socket_client;
