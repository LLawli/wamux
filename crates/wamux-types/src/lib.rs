//! The wamux domain's own types (#63): identifiers validated at construction,
//! closed enums for enum-like values, and `WamuxError` with its `tonic::Status`
//! mapping. Services convert the generated `pb::*` structs into these once, at
//! the boundary; `domain/` and `state/` work on these, not on the wire schema.

pub mod account;
pub mod error;
pub mod event_enums;
pub mod jid;
pub mod lid;
pub mod media_kind;
pub mod message_id;
pub mod messaging;
pub mod newsletter;
pub mod presence_state;
mod request_enum;

pub use account::{AccountId, AccountRef, ExternalRef};
pub use error::WamuxError;
pub use jid::{GroupJid, Jid, NewsletterJid, relay_jid};
pub use lid::LidPnQuery;
pub use media_kind::MediaKind;
pub use message_id::MessageId;
pub use messaging::{
    ContactCard, DownloadableMedia, EncryptedVote, InteractiveReply, LinkPreview, MessageTarget,
    NewPoll, OutgoingContext, OutgoingMedia, OutgoingText, POLL_SECRET_LEN, PollVoteCast,
    PollVotesToTally, QuotedRef, ReplyChoice, StatusMedia, StatusRevoke, StatusText,
};
pub use newsletter::{
    MAX_NEWSLETTER_VOTE_OPTIONS, NewsletterAddOnsQuery, NewsletterHistoryQuery, NewsletterPollVote,
};
pub use presence_state::PresenceState;

#[cfg(test)]
mod account_tests;
#[cfg(test)]
mod error_tests;
#[cfg(test)]
mod event_enums_tests;
#[cfg(test)]
mod jid_tests;
#[cfg(test)]
mod jid_wire_tests;
#[cfg(test)]
mod lid_tests;
#[cfg(test)]
mod media_kind_tests;
#[cfg(test)]
mod message_id_tests;
#[cfg(test)]
mod newsletter_tests;
#[cfg(test)]
mod presence_state_tests;
