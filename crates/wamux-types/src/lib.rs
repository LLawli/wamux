//! The wamux domain's own types (#63): identifiers validated at construction,
//! closed enums for enum-like values, and `WamuxError` with its `tonic::Status`
//! mapping. Services convert the generated `pb::*` structs into these once, at
//! the boundary; `domain/` and `state/` work on these, not on the wire schema.

pub mod account;
pub mod error;
pub mod jid;
pub mod media_kind;
pub mod message_id;

pub use account::{AccountId, AccountRef, ExternalRef};
pub use error::WamuxError;
pub use jid::{GroupJid, Jid, NewsletterJid};
pub use media_kind::MediaKind;
pub use message_id::MessageId;

#[cfg(test)]
mod account_tests;
#[cfg(test)]
mod error_tests;
#[cfg(test)]
mod jid_tests;
#[cfg(test)]
mod media_kind_tests;
#[cfg(test)]
mod message_id_tests;
