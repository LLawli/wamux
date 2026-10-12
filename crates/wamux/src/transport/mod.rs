//! Unix-socket transport lifecycle: bind/chmod the socket, gate peers by
//! `SO_PEERCRED` when configured, and drive graceful shutdown. No TCP, no TLS
//! (transport security is the filesystem's job).

pub mod peer_allowlist;
pub mod shutdown;
pub mod uds_listener;
