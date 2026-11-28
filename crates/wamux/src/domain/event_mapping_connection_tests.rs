//! Connection state, pairing, server acks, offline sync and calls.

use super::*;
use std::sync::Arc;
use std::time::Duration;

use wacore::types::call::{CallAction, IncomingCall};
use wacore::types::events::{
    ConnectFailureReason, Connected, Disconnected, LoggedOut, OfflineSyncCompleted,
    OfflineSyncInterrupted, OfflineSyncPreview, PairError, PairSuccess, PairingCode, PairingQrCode,
    QrScannedWithoutMultidevice, ServerAck, TempBanReason, TemporaryBan,
};
use whatsapp_rust::OwnedNodeRef;

use crate::proto::v1::event_envelope::Event as PbEvent;

use super::tests::{RAW_IQ_NODE, SENDER_JID, jid_of, map_one, mapped_connection, wire};

#[test]
fn maps_connected_and_disconnected_states() {
    let connected = mapped_connection(&Event::Connected(Connected::builder().build()));
    assert_eq!(connected.state, pb::ConnectionState::Connected as i32);
    // #126: neither typed info outside its own state.
    assert_eq!((connected.logged_out, connected.ban), (None, None));

    // Disconnected carries why the socket closed. The mapping ignores it (the
    // proto has one Disconnected state), so any reason exercises the same arm.
    let disconnected = mapped_connection(&Event::Disconnected(
        Disconnected::builder()
            .reason(wacore::net::DisconnectReason::StreamEnded)
            .build(),
    ));
    assert_eq!(disconnected.state, pb::ConnectionState::Disconnected as i32);
    assert_eq!((disconnected.logged_out, disconnected.ban), (None, None));
}

// #126: the reason is a typed LoggedOutInfo, not `Debug` inside `detail`.
#[test]
fn maps_logged_out_with_a_typed_reason() {
    // Any real `ConnectFailureReason` variant serves.
    // `LoggedOut` is boxed in the library (#30, upstream #1417).
    let c = mapped_connection(&Event::LoggedOut(Box::new(
        LoggedOut::builder()
            .on_connect(false)
            .reason(ConnectFailureReason::LoggedOut)
            .build(),
    )));
    assert_eq!(c.state, pb::ConnectionState::LoggedOut as i32);
    let info = c.logged_out.expect("LOGGED_OUT carries its info");
    assert_eq!(info.reason(), pb::LogoutReason::LoggedOut);
    assert_eq!(info.reason_code, 0, "a named reason carries no code");
    assert_eq!(c.ban, None);
}

#[test]
fn maps_temporary_ban_to_banned_state() {
    let c = mapped_connection(&Event::TemporaryBan(Box::new(
        TemporaryBan::builder()
            .code(TempBanReason::BlockedByUsers)
            .expire(std::time::Duration::from_secs(3_600))
            .build(),
    )));
    assert_eq!(c.state, pb::ConnectionState::Banned as i32);
    // #126: typed fields, not the whole TemporaryBan as `Debug` inside `detail`.
    let ban = c.ban.expect("BANNED carries its info");
    assert_eq!(ban.reason(), pb::BanReason::BlockedByUsers);
    assert_eq!(ban.reason_code, 0);
    assert_eq!(ban.expire_seconds, 3_600);
    assert_eq!(c.logged_out, None);
}

#[test]
fn maps_pair_success_to_paired_info() {
    let event = Event::PairSuccess(
        PairSuccess::builder()
            .id(jid_of(SENDER_JID))
            .lid(jid_of("204255232763170@lid"))
            .business_name("ACME Corp".to_string())
            .platform("smba".to_string())
            .build(),
    );
    match map_one(&event) {
        Some(PbEvent::Pairing(update)) => match update.event {
            Some(pb::pairing_update::Event::Paired(info)) => {
                assert_eq!(info.jid.expect("paired jid must be set").value, SENDER_JID);
                // business_name relays as itself; PairSuccess has no push name.
                assert_eq!(info.business_name, "ACME Corp");
                assert_eq!(
                    info.lid.expect("lid must be set").value,
                    "204255232763170@lid"
                );
                assert_eq!(info.platform, "smba");
            }
            other => panic!("expected paired info, got {other:?}"),
        },
        other => panic!("expected pairing, got {other:?}"),
    }
}

#[test]
fn maps_pair_error_with_message() {
    let event = Event::PairError(
        PairError::builder()
            .id(jid_of(SENDER_JID))
            .lid(jid_of("204255232763170@lid"))
            .business_name(String::new())
            .platform(String::new())
            .error("pair-device-timeout".to_string())
            .build(),
    );
    match map_one(&event) {
        Some(PbEvent::Pairing(update)) => {
            assert_eq!(
                update.event,
                Some(pb::pairing_update::Event::Error(pb::PairingError {
                    message: "pair-device-timeout".to_string(),
                }))
            );
        }
        other => panic!("expected pairing error, got {other:?}"),
    }
}

#[test]
fn drops_notification_and_raw_node_events() {
    let node = Arc::new(OwnedNodeRef::new(RAW_IQ_NODE.to_vec()).expect("valid wire node"));
    assert!(map_event(&Event::Notification(node.clone())).is_empty());
    assert!(map_event(&Event::RawNode(node)).is_empty());
}

#[test]
fn catch_all_maps_unmatched_variant_to_raw_event() {
    // QrScannedWithoutMultidevice has no explicit arm: it must land in Raw, not
    // vanish. One of the events kept in the catch-all on purpose (#141 triage).
    let event = Event::QrScannedWithoutMultidevice(QrScannedWithoutMultidevice::builder().build());
    match map_one(&event) {
        Some(PbEvent::Raw(raw)) => {
            assert_eq!(raw.kind, "QrScannedWithoutMultidevice");
            assert_eq!(raw.payload, serde_json::to_vec(&event).unwrap());
            let json: serde_json::Value = serde_json::from_slice(&raw.payload).unwrap();
            assert!(json["QrScannedWithoutMultidevice"].is_object());
            assert!(raw.note.is_empty());
        }
        other => panic!("expected raw event, got {other:?}"),
    }
}

#[test]
fn maps_pairing_qr_code() {
    let event = Event::PairingQrCode(
        PairingQrCode::builder()
            .code("QR-DATA".to_string())
            .timeout(Duration::from_secs(60))
            .build(),
    );
    match map_one(&event) {
        Some(pb::event_envelope::Event::Pairing(update)) => {
            assert_eq!(
                update.event,
                Some(pb::pairing_update::Event::QrCode("QR-DATA".into()))
            );
        }
        other => panic!("expected pairing qr, got {other:?}"),
    }
}

// REGRESSION (issue #4): a send whose device fan-out comes out empty answers
// with a real key and never reaches the server, which is indistinguishable from
// success on the SendResult alone. The ack is the only authority that can settle
// it, so it has to reach the edge as a typed event, not as the Raw catch-all.
#[test]
fn maps_server_ack_so_a_send_can_be_confirmed_against_the_server() {
    let event = Event::ServerAck(
        ServerAck::builder()
            .id("3EB0ABCDEF".to_string())
            .class("message".to_string())
            .from(jid_of(SENDER_JID))
            .timestamp(wacore::time::from_secs(1_717_932_222).unwrap())
            .build(),
    );
    match map_one(&event) {
        Some(PbEvent::ServerAck(a)) => {
            // Correlates with SendResult.key.id; that pairing is the whole point.
            assert_eq!(a.id, "3EB0ABCDEF");
            assert_eq!(a.class, "message");
            assert_eq!(a.from, wire(SENDER_JID));
            assert_eq!(a.timestamp, 1_717_932_222_000);
            // A plain ack is not a nack: the error field stays empty.
            assert!(a.error.is_empty());
        }
        other => panic!("expected server ack, got {other:?}"),
    }
}

// A nack is otherwise only a library log line. Relaying the code is what
// lets the edge tell "the server refused this" from "no ack yet".
#[test]
fn server_nack_relays_its_error_code() {
    let event = Event::ServerAck(
        ServerAck::builder()
            .id("3EB0FAILED".to_string())
            .class("message".to_string())
            .error("479".to_string())
            .build(),
    );
    match map_one(&event) {
        Some(PbEvent::ServerAck(a)) => {
            assert_eq!(a.error, "479");
            // Absent server attrs stay proto3 defaults, never a fake value; an
            // absent jid is an unset field (#122).
            assert_eq!(a.from, None);
            assert_eq!(a.timestamp, 0);
        }
        other => panic!("expected server ack, got {other:?}"),
    }
}

// REGRESSION (issue #11): a reconnect can arm a resume for hundreds of events
// and then be torn down mid-drain, leaving the backlog on the server. The two
// numbers that make that visible have to be typed, not buried in Raw.
#[test]
fn maps_offline_sync_preview_with_the_server_count() {
    let event = Event::OfflineSyncPreview(
        OfflineSyncPreview::builder()
            .total(711)
            .messages(600)
            .notifications(100)
            .receipts(11)
            .calls(0)
            .statuses(0)
            .app_data_changes(0)
            .build(),
    );
    match map_one(&event) {
        Some(PbEvent::OfflineSyncPreview(p)) => {
            assert_eq!(p.total, 711);
            assert_eq!(p.messages, 600);
            assert_eq!(p.notifications, 100);
            assert_eq!(p.receipts, 11);
        }
        other => panic!("expected offline sync preview, got {other:?}"),
    }
}

#[test]
fn maps_offline_sync_completed_with_what_was_delivered() {
    let event = Event::OfflineSyncCompleted(OfflineSyncCompleted::builder().count(5).build());
    match map_one(&event) {
        Some(PbEvent::OfflineSyncCompleted(c)) => assert_eq!(c.count, 5),
        other => panic!("expected offline sync completed, got {other:?}"),
    }
}

// Issue #38 (upstream #1380): the interrupted resume is typed like its two
// siblings, and never folded into OfflineSyncCompleted: "not caught up" is a
// different statement from "done".
#[test]
fn maps_offline_sync_interrupted_with_both_counts() {
    let event = Event::OfflineSyncInterrupted(
        OfflineSyncInterrupted::builder()
            .total(711)
            .delivered(300)
            .build(),
    );
    match map_one(&event) {
        Some(PbEvent::OfflineSyncInterrupted(i)) => {
            assert_eq!(i.total, 711);
            assert_eq!(i.delivered, 300);
        }
        other => panic!("expected offline sync interrupted, got {other:?}"),
    }
}

// The app-state updates are tested in `app_state_update_tests.rs` (#134), and
// every field of a call in `call_event_tests.rs` (#135).

#[test]
fn maps_incoming_call_offer_to_typed_call_event() {
    let event = Event::IncomingCall(Box::new(
        IncomingCall::builder()
            .from(jid_of(SENDER_JID))
            .stanza_id("STANZA-CALL-1".to_string())
            .timestamp(wacore::time::from_secs(1_717_932_000).unwrap())
            .offline(false)
            .action(CallAction::Offer {
                call_id: "CALL-ID-1".to_string(),
                call_creator: jid_of(SENDER_JID),
                caller_pn: None,
                caller_country_code: None,
                device_class: None,
                joinable: true,
                is_video: false,
                audio: vec![],
                group_jid: None,
            })
            .build(),
    ));
    match map_one(&event) {
        Some(PbEvent::Call(c)) => {
            assert_eq!(c.from, wire(SENDER_JID));
            // call_id is the CallAction id, NOT the stanza id.
            assert_eq!(c.call_id, "CALL-ID-1");
            // #135: the stanza id is its own field, not a key of a JSON blob.
            assert_eq!(c.stanza_id, "STANZA-CALL-1");
            assert!(matches!(c.action, Some(pb::call_event::Action::Offer(_))));
        }
        other => panic!("expected call event, got {other:?}"),
    }
}

#[test]
fn maps_incoming_call_terminate_action_token() {
    let event = Event::IncomingCall(Box::new(
        IncomingCall::builder()
            .from(jid_of(SENDER_JID))
            .stanza_id("STANZA-CALL-2".to_string())
            .timestamp(wacore::time::from_secs(1_717_932_000).unwrap())
            .offline(false)
            .action(CallAction::Terminate {
                call_id: "CALL-ID-2".to_string(),
                call_creator: jid_of(SENDER_JID),
                // Why the peer hung up. Absent = a plain missed call.
                reason: None,
                duration: Some(42),
                audio_duration: None,
            })
            .build(),
    ));
    match map_one(&event) {
        Some(PbEvent::Call(c)) => {
            assert_eq!(c.call_id, "CALL-ID-2");
            // #135: the oneof case names the action.
            assert!(matches!(
                c.action,
                Some(pb::call_event::Action::Terminate(_))
            ));
        }
        other => panic!("expected call event, got {other:?}"),
    }
}

#[test]
fn maps_pairing_code() {
    let event = Event::PairingCode(
        PairingCode::builder()
            .code("12345678".to_string())
            .timeout(Duration::from_secs(180))
            .build(),
    );
    match map_one(&event) {
        Some(pb::event_envelope::Event::Pairing(update)) => {
            assert_eq!(
                update.event,
                Some(pb::pairing_update::Event::PairCode("12345678".into()))
            );
        }
        other => panic!("expected pairing code, got {other:?}"),
    }
}
