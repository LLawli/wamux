//! The messaging domain's inputs (#63, #115): what a send, a reaction, a status
//! or a poll vote carries once the service has taken it off the wire.
//!
//! Each type converts from its generated `pb::*` struct once, at the service
//! boundary, so `domain/` never sees the wire schema. Routing fields (`account`,
//! `to`, `chat`) are not part of these: the service resolves them first, in the
//! order it always has. Every check a conversion makes is one the domain used
//! to make, with the same message, in the same order, plus the ones decided in
//! #115 (a jid that used to relay unparsed now parses; an empty id is refused).

pub mod key;
pub mod media;
pub mod polls;
pub mod rich;
pub mod status;
pub mod text;

pub use key::{MessageTarget, OutgoingContext, QuotedRef};
pub use media::{DownloadableMedia, OutgoingMedia};
pub use polls::{EncryptedVote, NewPoll, POLL_SECRET_LEN, PollVoteCast, PollVotesToTally};
pub use rich::{ContactCard, InteractiveReply, ReplyChoice};
pub use status::{StatusMedia, StatusRevoke, StatusText};
pub use text::{LinkPreview, OutgoingText};

#[cfg(test)]
mod key_tests;
#[cfg(test)]
mod media_tests;
#[cfg(test)]
mod polls_tests;
#[cfg(test)]
mod rich_tests;
#[cfg(test)]
mod status_tests;
#[cfg(test)]
mod text_tests;
