use super::*;
use std::time::Duration;

use bytes::Bytes;
use wacore::types::events::{
    ChatPresenceUpdate, ConnectFailureReason, Connected, DecryptFailMode, Disconnected,
    LazyHistorySync, LoggedOut, OfflineSyncCompleted, PairError, PairSuccess, PresenceUpdate,
    PushNameUpdate, Receipt, TempBanReason, TemporaryBan, UnavailableType, UndecryptableMessage,
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

fn mapped_media(msg: wa::Message) -> (pb::MediaDescriptor, String) {
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

#[test]
fn maps_image_media_descriptor_with_caption() {
    let (media, caption) = mapped_media(wa::Message {
        image_message: Some(Box::new(wa::message::ImageMessage {
            mimetype: Some("image/jpeg".to_string()),
            caption: Some("a cat".to_string()),
            file_length: Some(2048),
            direct_path: Some("/v/t62.image".to_string()),
            media_key: Some(vec![1, 2, 3]),
            file_enc_sha256: Some(vec![4, 5]),
            file_sha256: Some(vec![6, 7]),
            ..Default::default()
        })),
        ..Default::default()
    });
    assert_eq!(media.media_type, "image");
    assert_eq!(media.mime_type, "image/jpeg");
    assert_eq!(media.file_length, 2048);
    assert_eq!(media.direct_path, "/v/t62.image");
    assert_eq!(media.media_key, [1u8, 2, 3]);
    assert_eq!(media.file_enc_sha256, [4u8, 5]);
    assert_eq!(media.file_sha256, [6u8, 7]);
    assert_eq!(caption, "a cat");
}

#[test]
fn maps_video_media_descriptor_with_caption() {
    let (media, caption) = mapped_media(wa::Message {
        video_message: Some(Box::new(wa::message::VideoMessage {
            mimetype: Some("video/mp4".to_string()),
            caption: Some("a clip".to_string()),
            file_length: Some(2048),
            direct_path: Some("/v/t62.video".to_string()),
            ..Default::default()
        })),
        ..Default::default()
    });
    assert_eq!(media.media_type, "video");
    assert_eq!(media.mime_type, "video/mp4");
    assert_eq!(media.file_length, 2048);
    assert_eq!(media.direct_path, "/v/t62.video");
    assert_eq!(caption, "a clip");
}

#[test]
fn maps_audio_media_descriptor_without_caption() {
    let (media, caption) = mapped_media(wa::Message {
        audio_message: Some(Box::new(wa::message::AudioMessage {
            mimetype: Some("audio/ogg; codecs=opus".to_string()),
            file_length: Some(2048),
            direct_path: Some("/v/t62.audio".to_string()),
            ..Default::default()
        })),
        ..Default::default()
    });
    assert_eq!(media.media_type, "audio");
    assert_eq!(media.mime_type, "audio/ogg; codecs=opus");
    assert_eq!(media.file_length, 2048);
    assert_eq!(media.direct_path, "/v/t62.audio");
    // AudioMessage has no caption field in the WA proto.
    assert!(caption.is_empty());
}

#[test]
fn maps_document_media_descriptor_with_caption() {
    let (media, caption) = mapped_media(wa::Message {
        document_message: Some(Box::new(wa::message::DocumentMessage {
            mimetype: Some("application/pdf".to_string()),
            caption: Some("the invoice".to_string()),
            file_length: Some(2048),
            direct_path: Some("/v/t62.document".to_string()),
            ..Default::default()
        })),
        ..Default::default()
    });
    assert_eq!(media.media_type, "document");
    assert_eq!(media.mime_type, "application/pdf");
    assert_eq!(media.file_length, 2048);
    assert_eq!(media.direct_path, "/v/t62.document");
    assert_eq!(caption, "the invoice");
}

#[test]
fn maps_sticker_media_descriptor_without_caption() {
    let (media, caption) = mapped_media(wa::Message {
        sticker_message: Some(Box::new(wa::message::StickerMessage {
            mimetype: Some("image/webp".to_string()),
            file_length: Some(2048),
            direct_path: Some("/v/t62.sticker".to_string()),
            ..Default::default()
        })),
        ..Default::default()
    });
    assert_eq!(media.media_type, "sticker");
    assert_eq!(media.mime_type, "image/webp");
    assert_eq!(media.file_length, 2048);
    assert_eq!(media.direct_path, "/v/t62.sticker");
    // StickerMessage has no caption field in the WA proto.
    assert!(caption.is_empty());
}

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
            assert_eq!(r.r#type, "Read");
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
    assert_eq!(p.chat_state, "Composing");
    assert!(!p.chat_state.is_empty());
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
                assert_eq!(info.push_name, "ACME Corp");
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
