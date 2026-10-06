//! A channel history row's wacore-typed values as the contract's enums (#128).
//!
//! `PollType` and `EditAttribute` are closed library enums, matched with no
//! wildcard arm, so a variant added upstream fails the build at the bump. The
//! server's token survives next to UNKNOWN, because the core is a relay and
//! never drops what the server said. The four values typed by `whatsapp_rust`
//! (state, verification, role, message type) are matched in the daemon's
//! `domain::newsletters::channel_enums`: this crate does not depend on
//! whatsapp-rust (`scripts/check-crate-deps.sh`).

use wacore::types::message::{EditAttribute, PollType};
use wamux_proto::v1 as pb;

/// The row's `<meta polltype>`, read from the server's token on any row, and
/// the token verbatim when it is UNKNOWN (empty otherwise). `None` is
/// UNSPECIFIED.
pub fn poll_type_of(raw: Option<&str>) -> (pb::NewsletterPollType, String) {
    let Some(token) = raw else {
        return (pb::NewsletterPollType::Unspecified, String::new());
    };
    match PollType::try_from(token) {
        Ok(poll_type) => (named_poll_type(poll_type), String::new()),
        Err(_) => (pb::NewsletterPollType::Unknown, token.to_string()),
    }
}

/// No wildcard: `PollType` is closed, so a variant added upstream fails the
/// build at the bump instead of relaying as a guess (#128).
fn named_poll_type(poll_type: PollType) -> pb::NewsletterPollType {
    match poll_type {
        PollType::Creation => pb::NewsletterPollType::Creation,
        PollType::QuizCreation => pb::NewsletterPollType::QuizCreation,
        PollType::Vote => pb::NewsletterPollType::Vote,
        PollType::ResultSnapshot => pb::NewsletterPollType::ResultSnapshot,
        PollType::Edit => pb::NewsletterPollType::Edit,
    }
}

/// The row's `edit` attribute, and the token verbatim when it is UNKNOWN
/// (empty otherwise). `Empty` (absent) is UNSPECIFIED.
pub fn edit_attribute_of(edit: &EditAttribute) -> (pb::EditAttribute, String) {
    let named = match edit {
        EditAttribute::Empty => pb::EditAttribute::Unspecified,
        EditAttribute::MessageEdit => pb::EditAttribute::MessageEdit,
        EditAttribute::PinInChat => pb::EditAttribute::PinInChat,
        EditAttribute::AdminEdit => pb::EditAttribute::AdminEdit,
        EditAttribute::SenderRevoke => pb::EditAttribute::SenderRevoke,
        EditAttribute::AdminRevoke => pb::EditAttribute::AdminRevoke,
        EditAttribute::Unknown(token) => return (pb::EditAttribute::Unknown, token.clone()),
    };
    (named, String::new())
}
