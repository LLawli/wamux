//! wamux — WhatsApp multiplexer core daemon.
//!
//! Many WhatsApp accounts in one process, exposed over a single Unix-domain
//! gRPC socket. Relay-pure: only Signal/session state is persisted (Postgres or
//! SQLite, or Turso behind the `turso` cargo feature); no business message
//! history. Auth/permissions live in a separate edge.

pub mod cli;
pub mod config;
pub mod domain;
pub mod error;
pub mod observe;
/// The generated gRPC types live in `wamux-proto`; re-exported so every
/// `crate::proto::v1` / `wamux::proto::v1` path keeps resolving.
pub use wamux_proto as proto;
pub mod server;
pub mod services;
pub mod spool;
pub mod state;
pub mod storage;
/// Stress-test harness: a mock WhatsApp WSS server that real `whatsapp-rust`
/// clients connect to, so scale (hundreds of accounts) and the event pipeline
/// can be exercised without touching the real WhatsApp. Off by default (the
/// `stress` feature); never part of the production daemon.
#[cfg(feature = "stress")]
pub mod stress;
pub mod transport;
