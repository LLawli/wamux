//! #126: every value the library names maps to its own enum value, with no raw;
//! a value it does not name lands in UNKNOWN with the original token or code.

use wacore::types::events::{ConnectFailureReason, TempBanReason, UnavailableType};
use wacore::types::presence::{ChatPresence, ChatPresenceMedia, ReceiptType};
use wamux_proto::v1 as pb;

use crate::event_enums::{
    ban_reason_of, chat_state_of, logout_reason_of, receipt_type_of, unavailable_reason_of,
};

#[test]
fn every_receipt_type_maps_to_its_value() {
    use pb::ReceiptType as P;
    let table = [
        (ReceiptType::Delivered, P::Delivered),
        (ReceiptType::Sent, P::Sent),
        (ReceiptType::Sender, P::Sender),
        (ReceiptType::Retry, P::Retry),
        (ReceiptType::EncRekeyRetry, P::EncRekeyRetry),
        (ReceiptType::Read, P::Read),
        (ReceiptType::ReadSelf, P::ReadSelf),
        (ReceiptType::Played, P::Played),
        (ReceiptType::PlayedSelf, P::PlayedSelf),
        (ReceiptType::ServerError, P::ServerError),
        (ReceiptType::Inactive, P::Inactive),
        (ReceiptType::PeerMsg, P::PeerMsg),
        (ReceiptType::HistorySync, P::HistorySync),
    ];
    for (library, wire) in table {
        assert_eq!(
            receipt_type_of(&library),
            (wire, String::new()),
            "{library:?}"
        );
    }
}

// The relay never drops what the server said: an attribute the library does
// not name travels verbatim, beside UNKNOWN.
#[test]
fn an_other_receipt_type_is_unknown_with_its_token() {
    let other = ReceiptType::Other("future-kind".to_string());
    assert_eq!(
        receipt_type_of(&other),
        (pb::ReceiptType::Unknown, "future-kind".to_string())
    );
}

#[test]
fn every_chat_state_maps_to_its_value() {
    use pb::ChatState as P;
    let table = [
        (
            ChatPresence::Composing,
            ChatPresenceMedia::Text,
            P::Composing,
        ),
        (
            ChatPresence::Composing,
            ChatPresenceMedia::Audio,
            P::Recording,
        ),
        (ChatPresence::Paused, ChatPresenceMedia::Text, P::Paused),
        (ChatPresence::Paused, ChatPresenceMedia::Audio, P::Paused),
    ];
    for (state, media, wire) in table {
        assert_eq!(chat_state_of(state, media), wire, "{state:?}/{media:?}");
    }
}

#[test]
fn every_unavailable_reason_maps_to_its_value() {
    use pb::UnavailableReason as P;
    let table = [
        (UnavailableType::ViewOnce, P::ViewOnce),
        (UnavailableType::Hosted, P::Hosted),
        (UnavailableType::Bot, P::Bot),
    ];
    for (library, wire) in table {
        assert_eq!(unavailable_reason_of(library), wire, "{library:?}");
    }
}

// The library already collapsed the server's value into its own "unknown":
// there is nothing left to relay, so it is UNKNOWN with no raw field.
#[test]
fn an_unknown_unavailable_type_is_unknown() {
    assert_eq!(
        unavailable_reason_of(UnavailableType::Unknown),
        pb::UnavailableReason::Unknown
    );
}

#[test]
fn every_logout_reason_maps_to_its_value() {
    use pb::LogoutReason as P;
    let table = [
        (ConnectFailureReason::Generic, P::Generic),
        (ConnectFailureReason::LoggedOut, P::LoggedOut),
        (ConnectFailureReason::TempBanned, P::TempBanned),
        (ConnectFailureReason::AccountLocked, P::AccountLocked),
        (ConnectFailureReason::ClientOutdated, P::ClientOutdated),
        (ConnectFailureReason::UnknownLogout, P::UnknownLogout),
        (ConnectFailureReason::BadUserAgent, P::BadUserAgent),
        (ConnectFailureReason::CatExpired, P::CatExpired),
        (ConnectFailureReason::CatInvalid, P::CatInvalid),
        (ConnectFailureReason::NotFound, P::NotFound),
        (ConnectFailureReason::ClientUnknown, P::ClientUnknown),
        (
            ConnectFailureReason::InternalServerError,
            P::InternalServerError,
        ),
        (ConnectFailureReason::Experimental, P::Experimental),
        (
            ConnectFailureReason::ServiceUnavailable,
            P::ServiceUnavailable,
        ),
    ];
    for (library, wire) in table {
        assert_eq!(logout_reason_of(library), (wire, 0), "{library:?}");
    }
}

// 406 is a code the server sends ("unknown logout"), not a code wamux does not
// know: it must not be confused with UNKNOWN.
#[test]
fn an_unknown_logout_code_is_unknown_with_its_code() {
    assert_eq!(
        logout_reason_of(ConnectFailureReason::Unknown(416)),
        (pb::LogoutReason::Unknown, 416)
    );
    assert_ne!(
        logout_reason_of(ConnectFailureReason::UnknownLogout).0,
        pb::LogoutReason::Unknown
    );
}

#[test]
fn every_ban_reason_maps_to_its_value() {
    use pb::BanReason as P;
    let table = [
        (TempBanReason::SentToTooManyPeople, P::SentToTooManyPeople),
        (TempBanReason::BlockedByUsers, P::BlockedByUsers),
        (TempBanReason::CreatedTooManyGroups, P::CreatedTooManyGroups),
        (
            TempBanReason::SentTooManySameMessage,
            P::SentTooManySameMessage,
        ),
        (TempBanReason::BroadcastList, P::BroadcastList),
    ];
    for (library, wire) in table {
        assert_eq!(ban_reason_of(&library), (wire, 0), "{library:?}");
    }
}

#[test]
fn an_unknown_ban_code_is_unknown_with_its_code() {
    assert_eq!(
        ban_reason_of(&TempBanReason::Unknown(107)),
        (pb::BanReason::Unknown, 107)
    );
}
