//! Shared runtime state: the account registry, per-account handles, the
//! in-memory replay ring, and the event bridge that fans whatsapp-rust events
//! out to gRPC subscribers.

pub mod account_handle;
pub mod account_registry;
pub mod event_bridge;
pub mod event_ring;
pub mod send_echo;

pub use account_handle::{AccountHandle, RunningBot};
pub use account_registry::{AccountRegistry, RegistryTuning};
pub use event_ring::EventRing;
