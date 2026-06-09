//! Persistence. Relay-pure: only whatsapp-rust's Signal/session/device state is
//! stored. The Postgres backend implements wacore's four store traits.

pub mod postgres;
