//! wamux — WhatsApp multiplexer core daemon.
//!
//! Many WhatsApp accounts in one process, exposed over a single Unix-domain
//! gRPC socket. Relay-pure: only Signal/session state is persisted (Postgres);
//! no business message history. Auth/permissions live in a separate edge.

pub mod config;
pub mod domain;
pub mod error;
pub mod observe;
pub mod proto;
pub mod server;
pub mod services;
pub mod state;
pub mod storage;
/// Stress-test harness (mock WhatsApp WSS server). Off by default; never part of
/// the production daemon. See `docs/STATUS.md` Sprint 3.
#[cfg(feature = "stress")]
pub mod stress;
pub mod transport;
