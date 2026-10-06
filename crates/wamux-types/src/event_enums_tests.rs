//! #126: every value the library names maps to its own enum value, with no raw;
//! a value it does not name lands in UNKNOWN with the original token or code.

use wacore::types::call::{CallAction, VideoState};
use wacore::types::events::{ConnectFailureReason, TempBanReason, UnavailableType};
use wacore::types::group_call::{
    CallLinkMedia, GroupCallEncRekey, GroupCallUpdate, ScreenShare, ScreenShareState, WaitingRoom,
};
use wacore::types::presence::{ChatPresence, ChatPresenceMedia, ReceiptType};
use wacore_binary::Jid;
use wamux_proto::v1 as pb;

use crate::event_enums::{
    ban_reason_of, call_action_of, chat_state_of, logout_reason_of, receipt_type_of,
    unavailable_reason_of,
};

fn creator() -> Jid {
    "5511999000111@s.whatsapp.net"
        .parse()
        .expect("a test jid parses")
}

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

fn group_update() -> CallAction {
    let update = GroupCallUpdate::builder()
        .call_id("C".to_string())
        .call_creator(creator())
        .transaction_id(1)
        .media("audio".to_string())
        .connected_limit(8)
        .joinable(true)
        .av_upgradable(false)
        .rekey_requested(false)
        .participants(Vec::new())
        .build();
    CallAction::GroupUpdate {
        update: Box::new(update),
    }
}

fn enc_rekey() -> CallAction {
    let rekey = GroupCallEncRekey::builder()
        .call_id("C".to_string())
        .call_creator(creator())
        .transaction_id(1)
        .key_generation(1)
        .encryption_type("t".to_string())
        .encryption_version(1)
        .ciphertext(Vec::new())
        .build();
    CallAction::EncRekey {
        rekey: Box::new(rekey),
    }
}

fn waiting_room_update() -> CallAction {
    let room = WaitingRoom::builder()
        .call_id("C".to_string())
        .call_creator(creator())
        .link_token("L".to_string())
        .media(CallLinkMedia::Audio)
        .enabled(true)
        .is_admin(false)
        .users(Vec::new())
        .build();
    CallAction::WaitingRoomUpdate {
        room: Box::new(room),
    }
}

/// One value of each `CallAction` variant at the pinned rev, with the enum
/// value the contract gives it.
fn every_call_action() -> Vec<(CallAction, pb::CallActionKind)> {
    use pb::CallActionKind as P;
    let (id, by) = ("C".to_string(), creator());
    vec![
        (
            CallAction::Offer {
                call_id: id.clone(),
                call_creator: by.clone(),
                caller_pn: None,
                caller_country_code: None,
                device_class: None,
                joinable: true,
                is_video: false,
                audio: Vec::new(),
                group_jid: None,
            },
            P::Offer,
        ),
        (
            CallAction::OfferNotice {
                call_id: id.clone(),
                call_creator: by.clone(),
                is_video: false,
                is_group: false,
            },
            P::OfferNotice,
        ),
        (
            CallAction::PreAccept {
                call_id: id.clone(),
                call_creator: by.clone(),
                audio: Vec::new(),
            },
            P::PreAccept,
        ),
        (
            CallAction::Accept {
                call_id: id.clone(),
                call_creator: by.clone(),
                audio: Vec::new(),
            },
            P::Accept,
        ),
        (
            CallAction::Reject {
                call_id: id.clone(),
                call_creator: by.clone(),
                reason: None,
            },
            P::Reject,
        ),
        (
            CallAction::Terminate {
                call_id: id.clone(),
                call_creator: by.clone(),
                reason: None,
                duration: None,
                audio_duration: None,
            },
            P::Terminate,
        ),
        (
            CallAction::Transport {
                call_id: id.clone(),
                call_creator: by.clone(),
                p2p_cand_round: None,
                transport_message_type: None,
            },
            P::Transport,
        ),
        (
            CallAction::RelayLatency {
                call_id: id.clone(),
                call_creator: by.clone(),
            },
            P::RelayLatency,
        ),
        (
            CallAction::VideoState {
                call_id: id.clone(),
                call_creator: by.clone(),
                state: VideoState::Enabled,
                orientation: None,
                dec: None,
            },
            P::VideoState,
        ),
        (group_update(), P::GroupUpdate),
        (enc_rekey(), P::EncRekey),
        (waiting_room_update(), P::WaitingRoomUpdate),
        (
            CallAction::RaiseHand {
                call_id: id.clone(),
                call_creator: by.clone(),
                raised: true,
            },
            P::RaiseHand,
        ),
        (
            CallAction::ScreenShare {
                call_id: id,
                call_creator: by,
                screen_share: ScreenShare::builder()
                    .state(ScreenShareState::Started)
                    .version(1)
                    .build(),
            },
            P::ScreenShare,
        ),
    ]
}

#[test]
fn every_call_action_maps_to_its_value() {
    let actions = every_call_action();
    assert_eq!(
        actions.len(),
        14,
        "one per CallAction variant at the pinned rev"
    );
    for (library, wire) in actions {
        assert_eq!(call_action_of(&library).0, wire, "{}", library.wire_tag());
    }
}

// A named action carries no raw: `action_raw` is only for one the core does
// not name yet, and the eight that used to travel as a wire tag are named now.
#[test]
fn a_call_action_is_named_never_left_raw() {
    for (library, _) in every_call_action() {
        let (wire, raw) = call_action_of(&library);
        assert_ne!(wire, pb::CallActionKind::Unknown, "{}", library.wire_tag());
        assert_eq!(raw, "", "{}", library.wire_tag());
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
