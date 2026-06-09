//! Unix-socket transport lifecycle: bind/chmod the socket and drive graceful
//! shutdown. No TCP, no TLS (transport security is the filesystem's job).

pub mod shutdown;
pub mod uds_listener;
