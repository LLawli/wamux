use super::*;
use std::time::Duration;

use bytes::Bytes;
use wacore::types::call::{CallAction, IncomingCall};
use wacore::types::events::{
    ArchiveUpdate, ChatPresenceUpdate, ConnectFailureReason, Connected, DecryptFailMode,
    DeleteChatUpdate, Disconnected, LazyHistorySync, LoggedOut, MarkChatAsReadUpdate, MuteUpdate,
    OfflineSyncCompleted, PairError, PairSuccess, PinUpdate, PresenceUpdate, PushNameUpdate,
    Receipt, StarUpdate, TempBanReason, TemporaryBan, UnavailableType, UndecryptableMessage,
};
use wacore::types::message::MessageSource;
use wacore::types::presence::{ChatPresence, ChatPresenceMedia, ReceiptType};
use whatsapp_rust::{Jid, OwnedNodeRef};

use crate::proto::v1::event_envelope::Event as PbEvent;

const CHAT_JID: &str = "120363041234567890@g.us";
const SENDER_JID: &str = "5511999000111@s.whatsapp.net";

/// Minimal valid WABinary node `<iq/>`: LIST_8 list of size 1, then the tag
/// as a BINARY_8 string. Hand-rolled because whatsapp-rust's marshal helpers
/// are `#[cfg(test)]`-gated and wacore-binary is not a default wamux dep.
const RAW_IQ_NODE: &[u8] = &[0xF8, 0x01, 0xFC, 0x02, b'i', b'q'];

fn jid_of(value: &str) -> Jid {
    value.parse().unwrap()
}

fn sample_source() -> MessageSource {
    MessageSource {
        chat: jid_of(CHAT_JID),
        sender: jid_of(SENDER_JID),
        is_from_me: false,
        ..Default::default()
    }
}

fn sample_info() -> MessageInfo {
    MessageInfo {
        source: sample_source(),
        id: "WAMID-1".to_string(),
        push_name: "Alice".to_string(),
        // Fixed non-epoch instant: epoch (the Default) would hide a broken
        // timestamp mapping behind a zero.
        timestamp: wacore::time::from_secs(1_717_932_000).unwrap(),
        ..Default::default()
    }
}

fn mapped_inbound(msg: wa::Message) -> pb::InboundMessage {
    let event = Event::Message(Arc::new(msg), Arc::new(sample_info()));
    match map_event(&event) {
        Some(PbEvent::Message(m)) => m,
        other => panic!("expected inbound message, got {other:?}"),
    }
}

/// pub(super): the media-descriptor suite lives in a sibling test file
/// (event_mapping_media_tests.rs) to keep both under the 500-line rule.
pub(super) fn mapped_media(msg: wa::Message) -> (pb::MediaDescriptor, String) {
    let out = mapped_inbound(msg);
    let media = out
        .media
        .expect("media sub-message must yield a descriptor");
    (media, out.caption)
}

fn mapped_connection(event: &Event) -> pb::ConnectionStateChanged {
    match map_event(event) {
        Some(PbEvent::Connection(c)) => c,
        other => panic!("expected connection event, got {other:?}"),
    }
}

fn mapped_presence(event: &Event) -> pb::PresenceUpdate {
    match map_event(event) {
        Some(PbEvent::Presence(p)) => p,
        other => panic!("expected presence, got {other:?}"),
    }
}

#[test]
fn maps_conversation_text_message_with_key_and_metadata() {
    let out = mapped_inbound(wa::Message {
        conversation: Some("hello world".to_string()),
        ..Default::default()
    });
    assert_eq!(out.text, "hello world");
    assert_eq!(out.chat, CHAT_JID);
    assert_eq!(out.sender, SENDER_JID);
    assert_eq!(out.push_name, "Alice");
    assert_eq!(out.timestamp, 1_717_932_000_000);

    let key = out.key.expect("inbound message must carry a key");
    assert_eq!(key.remote_jid, CHAT_JID);
    assert_eq!(key.id, "WAMID-1");
    assert!(!key.from_me);
    assert_eq!(key.participant, SENDER_JID);

    // raw_message is the full encoded wa.Message; decoding restores the text.
    assert!(!out.raw_message.is_empty());
    let decoded = wa::Message::decode(out.raw_message.as_slice()).unwrap();
    assert_eq!(decoded.conversation.as_deref(), Some("hello world"));
}

#[test]
fn maps_extended_text_with_mentions_and_quote() {
    let out = mapped_inbound(wa::Message {
        extended_text_message: Some(Box::new(wa::message::ExtendedTextMessage {
            text: Some("hey @55117770001".to_string()),
            context_info: Some(Box::new(wa::ContextInfo {
                mentioned_jid: vec!["55117770001@s.whatsapp.net".to_string()],
                stanza_id: Some("QUOTED-STANZA-1".to_string()),
                participant: Some("55116660002@s.whatsapp.net".to_string()),
                ..Default::default()
            })),
            ..Default::default()
        })),
        ..Default::default()
    });
    assert_eq!(out.text, "hey @55117770001");
    assert_eq!(
        out.mentions,
        [pb::Mention {
            jid: "55117770001@s.whatsapp.net".to_string(),
        }]
    );
    let quote = out.quote.expect("stanza_id must produce a quote");
    assert_eq!(quote.participant, "55116660002@s.whatsapp.net");
    let quoted = quote.quoted.expect("quote must carry the quoted key");
    assert_eq!(quoted.id, "QUOTED-STANZA-1");
    assert_eq!(quoted.remote_jid, CHAT_JID);
    assert_eq!(quoted.participant, "55116660002@s.whatsapp.net");
    assert!(!quoted.from_me);
}

#[test]
fn maps_reaction_message_with_target_key() {
    let out = mapped_inbound(wa::Message {
        reaction_message: Some(wa::message::ReactionMessage {
            key: Some(wa::MessageKey {
                remote_jid: Some(CHAT_JID.to_string()),
                id: Some("TARGET-MSG-1".to_string()),
                from_me: Some(true),
                participant: Some(SENDER_JID.to_string()),
            }),
            // Reaction payloads are emoji on the wire by protocol.
            text: Some("\u{1F44D}".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    });
    assert_eq!(out.reaction, "\u{1F44D}");
    let target = out
        .reaction_target
        .expect("reaction key must map to a target");
    assert_eq!(target.remote_jid, CHAT_JID);
    assert_eq!(target.id, "TARGET-MSG-1");
    assert!(target.from_me);
    assert_eq!(target.participant, SENDER_JID);
}

// The five media-descriptor tests live in event_mapping_media_tests.rs.

#[test]
fn maps_receipt_with_ids_type_and_millis() {
    let event = Event::Receipt(Receipt {
        source: sample_source(),
        message_ids: vec!["AAA111".to_string(), "BBB222".to_string()],
        timestamp: wacore::time::from_secs(1_717_932_111).unwrap(),
        r#type: ReceiptType::Read,
    });
    match map_event(&event) {
        Some(PbEvent::Receipt(r)) => {
            assert_eq!(r.chat, CHAT_JID);
            assert_eq!(r.sender, SENDER_JID);
            assert_eq!(r.message_ids, ["AAA111", "BBB222"]);
            // Lowercase wire token per the proto contract, never Debug casing.
            assert_eq!(r.r#type, "read");
            assert_eq!(r.timestamp, 1_717_932_111_000);
        }
        other => panic!("expected receipt, got {other:?}"),
    }
}

#[test]
fn maps_undecryptable_message_with_reason() {
    let event = Event::UndecryptableMessage(UndecryptableMessage {
        info: Arc::new(sample_info()),
        is_unavailable: true,
        unavailable_type: UnavailableType::ViewOnce,
        decrypt_fail_mode: DecryptFailMode::Hide,
    });
    match map_event(&event) {
        Some(PbEvent::Undecryptable(u)) => {
            assert_eq!(u.chat, CHAT_JID);
            assert_eq!(u.sender, SENDER_JID);
            assert_eq!(u.reason, "ViewOnce");
        }
        other => panic!("expected undecryptable, got {other:?}"),
    }
}

#[test]
fn maps_presence_offline_with_last_seen_seconds() {
    let p = mapped_presence(&Event::Presence(PresenceUpdate {
        from: jid_of(SENDER_JID),
        unavailable: true,
        last_seen: Some(wacore::time::from_secs(1_717_932_000).unwrap()),
    }));
    assert_eq!(p.jid, SENDER_JID);
    // "unavailable" on the wire means offline for the edge: the flag inverts.
    assert!(!p.online);
    assert_eq!(p.last_seen, 1_717_932_000);
    assert!(p.chat_state.is_empty());
}

#[test]
fn maps_presence_online_without_last_seen() {
    let p = mapped_presence(&Event::Presence(PresenceUpdate {
        from: jid_of(SENDER_JID),
        unavailable: false,
        last_seen: None,
    }));
    assert!(p.online);
    assert_eq!(p.last_seen, 0);
}

#[test]
fn maps_chat_presence_to_composing_state() {
    let p = mapped_presence(&Event::ChatPresence(ChatPresenceUpdate {
        source: sample_source(),
        state: ChatPresence::Composing,
        media: ChatPresenceMedia::Text,
    }));
    assert_eq!(p.jid, SENDER_JID);
    // A contact that is typing is by definition online.
    assert!(p.online);
    // Lowercase wire token per the proto contract (composing|recording|paused),
    // round-trippable into SendPresenceRequest.state.
    assert_eq!(p.chat_state, "composing");
}

// The lib has no Recording variant: it models recording as Composing with
// media=Audio. The wire contract flattens that pair back to "recording".
#[test]
fn maps_chat_presence_audio_to_recording_state() {
    let p = mapped_presence(&Event::ChatPresence(ChatPresenceUpdate {
        source: sample_source(),
        state: ChatPresence::Composing,
        media: ChatPresenceMedia::Audio,
    }));
    assert_eq!(p.chat_state, "recording");
}

#[test]
fn maps_push_name_update() {
    let event = Event::PushNameUpdate(PushNameUpdate {
        jid: jid_of(SENDER_JID),
        message: Box::new(sample_info()),
        old_push_name: "Old Alice".to_string(),
        new_push_name: "New Alice".to_string(),
    });
    match map_event(&event) {
        Some(PbEvent::PushName(p)) => {
            assert_eq!(p.jid, SENDER_JID);
            assert_eq!(p.push_name, "New Alice");
        }
        other => panic!("expected push name, got {other:?}"),
    }
}

#[test]
fn maps_history_sync_with_raw_passthrough() {
    let sync_type = wa::history_sync::HistorySyncType::Recent as i32;
    let proto = wa::HistorySync {
        sync_type,
        conversations: vec![wa::Conversation {
            id: CHAT_JID.to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let raw = proto.encode_to_vec();
    let lazy = LazyHistorySync::new(Bytes::from(raw.clone()), sync_type, Some(3), Some(75))
        .with_peer_data_request_session_id(Some("sess-1".to_string()));
    match map_event(&Event::HistorySync(Box::new(lazy))) {
        Some(PbEvent::HistorySync(h)) => {
            assert_eq!(h.sync_type, sync_type);
            assert_eq!(h.chunk_order, Some(3));
            assert_eq!(h.progress, Some(75));
            assert_eq!(h.session_id.as_deref(), Some("sess-1"));
            // Verbatim relay: the edge must be able to decode `raw` itself.
            assert_eq!(h.raw, raw);
            let decoded = wa::HistorySync::decode(h.raw.as_slice()).unwrap();
            assert_eq!(decoded.sync_type, sync_type);
            assert_eq!(decoded.conversations[0].id, CHAT_JID);
        }
        other => panic!("expected history sync, got {other:?}"),
    }
}

#[test]
fn maps_connected_and_disconnected_states() {
    let connected = mapped_connection(&Event::Connected(Connected));
    assert_eq!(connected.state, pb::ConnectionState::Connected as i32);
    assert!(connected.detail.is_empty());

    let disconnected = mapped_connection(&Event::Disconnected(Disconnected));
    assert_eq!(disconnected.state, pb::ConnectionState::Disconnected as i32);
    assert!(disconnected.detail.is_empty());
}

#[test]
fn maps_logged_out_with_reason_in_detail() {
    let c = mapped_connection(&Event::LoggedOut(LoggedOut {
        on_connect: false,
        reason: ConnectFailureReason::MainDeviceGone,
    }));
    assert_eq!(c.state, pb::ConnectionState::LoggedOut as i32);
    assert!(
        c.detail.contains("MainDeviceGone"),
        "detail was {:?}",
        c.detail
    );
}

#[test]
fn maps_temporary_ban_to_banned_state() {
    // chrono::Duration without a direct chrono dep: subtract two DateTimes.
    let start = wacore::time::from_secs(0).unwrap();
    let end = wacore::time::from_secs(3_600).unwrap();
    let c = mapped_connection(&Event::TemporaryBan(TemporaryBan {
        code: TempBanReason::BlockedByUsers,
        expire: end - start,
    }));
    assert_eq!(c.state, pb::ConnectionState::Banned as i32);
    assert!(
        c.detail.contains("BlockedByUsers"),
        "detail was {:?}",
        c.detail
    );
}

#[test]
fn maps_pair_success_to_paired_info() {
    let event = Event::PairSuccess(PairSuccess {
        id: jid_of(SENDER_JID),
        lid: jid_of("204255232763170@lid"),
        business_name: "ACME Corp".to_string(),
        platform: "smba".to_string(),
    });
    match map_event(&event) {
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
    let event = Event::PairError(PairError {
        id: jid_of(SENDER_JID),
        lid: jid_of("204255232763170@lid"),
        business_name: String::new(),
        platform: String::new(),
        error: "pair-device-timeout".to_string(),
    });
    match map_event(&event) {
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
    assert!(map_event(&Event::Notification(node.clone())).is_none());
    assert!(map_event(&Event::RawNode(node)).is_none());
}

#[test]
fn catch_all_maps_unmatched_variant_to_raw_event() {
    // OfflineSyncCompleted has no explicit arm: it must land in Raw, not vanish.
    let event = Event::OfflineSyncCompleted(OfflineSyncCompleted { count: 7 });
    match map_event(&event) {
        Some(PbEvent::Raw(raw)) => {
            assert_eq!(raw.kind, "OfflineSyncCompleted");
            assert_eq!(raw.payload, serde_json::to_vec(&event).unwrap());
            let json: serde_json::Value = serde_json::from_slice(&raw.payload).unwrap();
            assert_eq!(json["OfflineSyncCompleted"]["count"], 7);
            assert!(raw.note.is_empty());
        }
        other => panic!("expected raw event, got {other:?}"),
    }
}

#[test]
fn maps_pairing_qr_code() {
    let event = Event::PairingQrCode {
        code: "QR-DATA".to_string(),
        timeout: Duration::from_secs(60),
    };
    match map_event(&event) {
        Some(pb::event_envelope::Event::Pairing(update)) => {
            assert_eq!(
                update.event,
                Some(pb::pairing_update::Event::QrCode("QR-DATA".into()))
            );
        }
        other => panic!("expected pairing qr, got {other:?}"),
    }
}

fn mapped_app_state(event: &Event) -> pb::AppStateUpdate {
    match map_event(event) {
        Some(PbEvent::AppState(s)) => s,
        other => panic!("expected app-state update, got {other:?}"),
    }
}

#[test]
fn maps_archive_update_to_typed_app_state() {
    let s = mapped_app_state(&Event::ArchiveUpdate(ArchiveUpdate {
        jid: jid_of(CHAT_JID),
        timestamp: wacore::time::from_secs(1_717_932_000).unwrap(),
        action: Box::default(),
        from_full_sync: false,
    }));
    assert_eq!(s.chat, CHAT_JID);
    assert_eq!(s.kind, "archive");
    // raw is the verbatim serde_json of the lib struct: the edge decodes it.
    // Jid serializes structurally (user/server/...), so we assert the user part
    // rather than a flat jid string.
    assert!(!s.raw.is_empty());
    let json: serde_json::Value = serde_json::from_slice(&s.raw).unwrap();
    assert_eq!(json["jid"]["user"], "120363041234567890");
    assert_eq!(json["jid"]["server"], "g.us");
}

#[test]
fn maps_pin_update_to_typed_app_state() {
    let s = mapped_app_state(&Event::PinUpdate(PinUpdate {
        jid: jid_of(CHAT_JID),
        timestamp: wacore::time::from_secs(1_717_932_000).unwrap(),
        action: Box::default(),
        from_full_sync: false,
    }));
    assert_eq!(s.chat, CHAT_JID);
    assert_eq!(s.kind, "pin");
}

#[test]
fn maps_mute_update_to_typed_app_state() {
    let s = mapped_app_state(&Event::MuteUpdate(MuteUpdate {
        jid: jid_of(CHAT_JID),
        timestamp: wacore::time::from_secs(1_717_932_000).unwrap(),
        action: Box::default(),
        from_full_sync: false,
    }));
    assert_eq!(s.chat, CHAT_JID);
    assert_eq!(s.kind, "mute");
}

// StarUpdate names its chat `chat_jid` (it points at a message); the mapping
// must read that field, not the missing `jid`.
#[test]
fn maps_star_update_reads_chat_jid() {
    let s = mapped_app_state(&Event::StarUpdate(StarUpdate {
        chat_jid: jid_of(CHAT_JID),
        participant_jid: None,
        message_id: "WAMID-STAR".to_string(),
        from_me: false,
        timestamp: wacore::time::from_secs(1_717_932_000).unwrap(),
        action: Box::default(),
        from_full_sync: false,
    }));
    assert_eq!(s.chat, CHAT_JID);
    assert_eq!(s.kind, "star");
}

#[test]
fn maps_mark_chat_as_read_update_to_typed_app_state() {
    let s = mapped_app_state(&Event::MarkChatAsReadUpdate(MarkChatAsReadUpdate {
        jid: jid_of(CHAT_JID),
        timestamp: wacore::time::from_secs(1_717_932_000).unwrap(),
        action: Box::default(),
        from_full_sync: false,
    }));
    assert_eq!(s.chat, CHAT_JID);
    assert_eq!(s.kind, "mark_read");
}

#[test]
fn maps_delete_chat_update_to_typed_app_state() {
    let s = mapped_app_state(&Event::DeleteChatUpdate(DeleteChatUpdate {
        jid: jid_of(CHAT_JID),
        delete_media: false,
        timestamp: wacore::time::from_secs(1_717_932_000).unwrap(),
        action: Box::default(),
        from_full_sync: false,
    }));
    assert_eq!(s.chat, CHAT_JID);
    assert_eq!(s.kind, "delete_chat");
}

#[test]
fn maps_incoming_call_offer_to_typed_call_event() {
    let event = Event::IncomingCall(IncomingCall {
        from: jid_of(SENDER_JID),
        stanza_id: "STANZA-CALL-1".to_string(),
        notify: None,
        platform: None,
        version: None,
        timestamp: wacore::time::from_secs(1_717_932_000).unwrap(),
        offline: false,
        action: CallAction::Offer {
            call_id: "CALL-ID-1".to_string(),
            call_creator: jid_of(SENDER_JID),
            caller_pn: None,
            caller_country_code: None,
            device_class: None,
            joinable: true,
            is_video: false,
            audio: vec![],
            group_jid: None,
        },
    });
    match map_event(&event) {
        Some(PbEvent::Call(c)) => {
            assert_eq!(c.from, SENDER_JID);
            // call_id is the CallAction id, NOT the stanza id.
            assert_eq!(c.call_id, "CALL-ID-1");
            assert_eq!(c.action, "offer");
            assert!(!c.raw.is_empty());
            let json: serde_json::Value = serde_json::from_slice(&c.raw).unwrap();
            assert_eq!(json["stanza_id"], "STANZA-CALL-1");
        }
        other => panic!("expected call event, got {other:?}"),
    }
}

#[test]
fn maps_incoming_call_terminate_action_token() {
    let event = Event::IncomingCall(IncomingCall {
        from: jid_of(SENDER_JID),
        stanza_id: "STANZA-CALL-2".to_string(),
        notify: None,
        platform: None,
        version: None,
        timestamp: wacore::time::from_secs(1_717_932_000).unwrap(),
        offline: false,
        action: CallAction::Terminate {
            call_id: "CALL-ID-2".to_string(),
            call_creator: jid_of(SENDER_JID),
            duration: Some(42),
            audio_duration: None,
        },
    });
    match map_event(&event) {
        Some(PbEvent::Call(c)) => {
            assert_eq!(c.call_id, "CALL-ID-2");
            // Lowercase wire token, never Debug casing.
            assert_eq!(c.action, "terminate");
        }
        other => panic!("expected call event, got {other:?}"),
    }
}

#[test]
fn maps_pairing_code() {
    let event = Event::PairingCode {
        code: "12345678".to_string(),
        timeout: Duration::from_secs(180),
    };
    match map_event(&event) {
        Some(pb::event_envelope::Event::Pairing(update)) => {
            assert_eq!(
                update.event,
                Some(pb::pairing_update::Event::PairCode("12345678".into()))
            );
        }
        other => panic!("expected pairing code, got {other:?}"),
    }
}
