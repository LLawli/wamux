//! Persistence. Relay-pure: only whatsapp-rust's Signal/session/device state is
//! stored, never business message history.
//!
//! `StorageEngine` is the abstraction; `postgres` is its implementation,
//! implementing wacore's four store traits on a device-scoped backend type.

pub mod blob_codec;
pub mod engine;
pub mod postgres;
pub mod sqlx_error;

pub use engine::{AccountRow, StorageEngine};
