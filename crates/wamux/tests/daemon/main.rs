//! Daemon integration tests: one binary for every suite that runs the real
//! server over a Unix socket (startup, events, shutdown, reflection, peer
//! checks), so they link once instead of once each (#235).
//!
//! A new daemon suite goes here as a `mod`. A suite that owns process-global
//! state (the process umask, a global subscriber or recorder, the allocator)
//! must stay its own binary: in a shared one it would see the other suites'
//! state. `socket_bind_umask` is one.

// Only a subset of the shared helpers is used per suite.
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;

mod daemon_umask;
mod event_order;
mod event_subscription;
mod graceful_shutdown;
mod grpc_server;
mod peer_allowlist;
mod reflection;
mod socket_bind;
mod typed_events;
