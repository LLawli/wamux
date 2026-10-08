//! The library's enum-like event values as the contract's enums (#73, #126).
//!
//! Each one is an explicit `match`, written once, so a rename in whatsapp-rust
//! cannot change the wire the way a `Debug` string did. Where the library can
//! hand over a value wamux does not name (`#[non_exhaustive]` enums, or an
//! `Unknown(code)` fallback), the result is UNKNOWN plus the original value,
//! because the core is a relay and never drops what the server said. Where the
//! library's set is closed, the match has no wildcard arm: a variant added
//! upstream fails the build at the bump instead of reaching the edge unnamed.

use wacore::types::events::{ConnectFailureReason, TempBanReason, UnavailableType};
use wacore::types::presence::{ChatPresence, ChatPresenceMedia, ReceiptType};
use wamux_proto::v1 as pb;

/// The receipt's type, and the stanza attribute verbatim when it is UNKNOWN
/// (empty otherwise).
pub fn receipt_type_of(receipt: &ReceiptType) -> (pb::ReceiptType, String) {
    let known = match receipt {
        ReceiptType::Delivered => pb::ReceiptType::Delivered,
        ReceiptType::Sent => pb::ReceiptType::Sent,
        ReceiptType::Sender => pb::ReceiptType::Sender,
        ReceiptType::Retry => pb::ReceiptType::Retry,
        ReceiptType::EncRekeyRetry => pb::ReceiptType::EncRekeyRetry,
        ReceiptType::Read => pb::ReceiptType::Read,
        ReceiptType::ReadSelf => pb::ReceiptType::ReadSelf,
        ReceiptType::Played => pb::ReceiptType::Played,
        ReceiptType::PlayedSelf => pb::ReceiptType::PlayedSelf,
        ReceiptType::ServerError => pb::ReceiptType::ServerError,
        ReceiptType::Inactive => pb::ReceiptType::Inactive,
        ReceiptType::PeerMsg => pb::ReceiptType::PeerMsg,
        ReceiptType::HistorySync => pb::ReceiptType::HistorySync,
        ReceiptType::Other(raw) => return (pb::ReceiptType::Unknown, raw.clone()),
        // `#[non_exhaustive]`: a variant added upstream is not named here yet,
        // so it relays under the library's own wire string, never dropped.
        other => return (pb::ReceiptType::Unknown, other.as_wire_str().to_string()),
    };
    (known, String::new())
}

/// The chat state the edge draws. The library models "recording" as composing
/// with audio media.
pub fn chat_state_of(state: ChatPresence, media: ChatPresenceMedia) -> pb::ChatState {
    match (state, media) {
        (ChatPresence::Composing, ChatPresenceMedia::Audio) => pb::ChatState::Recording,
        (ChatPresence::Composing, ChatPresenceMedia::Text) => pb::ChatState::Composing,
        (ChatPresence::Paused, _) => pb::ChatState::Paused,
    }
}

/// Why a message could not be decrypted. Closed set, no wildcard arm (#126).
pub fn unavailable_reason_of(reason: UnavailableType) -> pb::UnavailableReason {
    match reason {
        UnavailableType::Unknown => pb::UnavailableReason::Unknown,
        UnavailableType::ViewOnce => pb::UnavailableReason::ViewOnce,
        UnavailableType::Hosted => pb::UnavailableReason::Hosted,
        UnavailableType::Bot => pb::UnavailableReason::Bot,
    }
}

/// The logout reason, and the server's code when it is UNKNOWN (0 otherwise).
/// Closed set, no wildcard arm (#126).
pub fn logout_reason_of(reason: ConnectFailureReason) -> (pb::LogoutReason, i32) {
    let known = match reason {
        ConnectFailureReason::Generic => pb::LogoutReason::Generic,
        ConnectFailureReason::LoggedOut => pb::LogoutReason::LoggedOut,
        ConnectFailureReason::TempBanned => pb::LogoutReason::TempBanned,
        ConnectFailureReason::AccountLocked => pb::LogoutReason::AccountLocked,
        ConnectFailureReason::UnknownLogout => pb::LogoutReason::UnknownLogout,
        ConnectFailureReason::ClientOutdated => pb::LogoutReason::ClientOutdated,
        ConnectFailureReason::BadUserAgent => pb::LogoutReason::BadUserAgent,
        ConnectFailureReason::CatExpired => pb::LogoutReason::CatExpired,
        ConnectFailureReason::CatInvalid => pb::LogoutReason::CatInvalid,
        ConnectFailureReason::NotFound => pb::LogoutReason::NotFound,
        ConnectFailureReason::ClientUnknown => pb::LogoutReason::ClientUnknown,
        ConnectFailureReason::InternalServerError => pb::LogoutReason::InternalServerError,
        ConnectFailureReason::Experimental => pb::LogoutReason::Experimental,
        ConnectFailureReason::ServiceUnavailable => pb::LogoutReason::ServiceUnavailable,
        ConnectFailureReason::Unknown(code) => return (pb::LogoutReason::Unknown, code),
    };
    (known, 0)
}

/// The temporary-ban reason, and the server's code when it is UNKNOWN (0
/// otherwise). Closed set, no wildcard arm (#126).
pub fn ban_reason_of(reason: &TempBanReason) -> (pb::BanReason, i32) {
    let known = match reason {
        TempBanReason::SentToTooManyPeople => pb::BanReason::SentToTooManyPeople,
        TempBanReason::BlockedByUsers => pb::BanReason::BlockedByUsers,
        TempBanReason::CreatedTooManyGroups => pb::BanReason::CreatedTooManyGroups,
        TempBanReason::SentTooManySameMessage => pb::BanReason::SentTooManySameMessage,
        TempBanReason::BroadcastList => pb::BanReason::BroadcastList,
        TempBanReason::Unknown(code) => return (pb::BanReason::Unknown, *code),
    };
    (known, 0)
}
