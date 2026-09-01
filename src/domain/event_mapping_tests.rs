use super::*;
use std::time::Duration;

use bytes::Bytes;
use wacore::types::call::{CallAction, IncomingCall};
use wacore::types::events::{
    ArchiveUpdate, BatchOrigin, ChatPresenceUpdate, ConnectFailureReason, Connected,
    DecryptFailMode, DeleteChatUpdate, Disconnected, InboundMessage, LazyHistorySync, LoggedOut,
    MarkChatAsReadUpdate, MessageBatch, MuteUpdate, OfflineSyncCompleted, OfflineSyncPreview,
    PairError, PairSuccess, PairingCode, PairingCodeRefresh, PairingQrCode, PinUpdate,
    PresenceUpdate, PushNameUpdate, Receipt, ServerAck, StarUpdate, TempBanReason, TemporaryBan,
    UnavailableType, UndecryptableMessage,
};
use wacore::types::message::MessageSource;
use wacore::types::presence::{ChatPresence, ChatPresenceMedia, ReceiptType};
use whatsapp_rust::buffa::MessageField;
use whatsapp_rust::{Jid, OwnedNodeRef};

use crate::proto::v1::event_envelope::Event as PbEvent;

const CHAT_JID: &str = "120363041234567890@g.us";
const SENDER_JID: &str = "5511999000111@s.whatsapp.net";
const LID_JID: &str = "169815004184633@lid";

/// Minimal valid WABinary node `<iq/>`: LIST_8 list of size 1, then the tag
/// as a BINARY_8 string. Hand-rolled because whatsapp-rust's marshal helpers
/// are `#[cfg(test)]`-gated and wacore-binary is not a default wamux dep.
const RAW_IQ_NODE: &[u8] = &[0xF8, 0x01, 0xFC, 0x02, b'i', b'q'];

/// zlib-compress a history-sync payload the way the server does, so the lazy
/// handle can inflate it back. `flate2` is already in the tree (via the lib).
fn zlib_compress(raw: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(raw).expect("in-memory write cannot fail");
    encoder.finish().expect("in-memory finish cannot fail")
}

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

/// 0.7 delivers inbound messages in batches, so `map_event` returns a list.
/// Every event these tests build maps to exactly one payload; asserting that
/// here keeps each test's own assertion about the payload, not about arity.
fn map_one(event: &Event) -> Option<PbEvent> {
    let mut mapped = map_event(event);
    assert!(
        mapped.len() <= 1,
        "this helper is for single-payload events; got {}",
        mapped.len()
    );
    mapped.pop()
}

/// One inbound message wrapped in the batch-of-one the lib dispatches for live
/// traffic. Both payload structs are sealed in 0.7, hence the builders.
fn message_event(msg: wa::Message, info: MessageInfo) -> Event {
    Event::Messages(
        MessageBatch::builder()
            .messages(Arc::from(vec![
                InboundMessage::builder()
                    .message(Arc::new(msg))
                    .info(Arc::new(info))
                    .build(),
            ]))
            .origin(BatchOrigin::Live)
            .build(),
    )
}

fn mapped_inbound(msg: wa::Message) -> pb::InboundMessage {
    match map_one(&message_event(msg, sample_info())) {
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
    match map_one(event) {
        Some(PbEvent::Connection(c)) => c,
        other => panic!("expected connection event, got {other:?}"),
    }
}

fn mapped_presence(event: &Event) -> pb::PresenceUpdate {
    match map_one(event) {
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
    let decoded = wa::Message::decode_from_slice(out.raw_message.as_slice()).unwrap();
    assert_eq!(decoded.conversation.as_deref(), Some("hello world"));
}

// REGRESSION (issue #1): a `@lid` sender is unidentifiable on its own, and the
// stanza already carries the phone side in `sender_alt`. The core dropped it,
// forcing the edge to poll for an identity the event was holding.
#[test]
fn relays_the_stanza_alt_jids_of_a_lid_sender() {
    let source = MessageSource {
        chat: jid_of(LID_JID),
        sender: jid_of(LID_JID),
        sender_alt: Some(jid_of(SENDER_JID)),
        recipient_alt: Some(jid_of("5511888000222@s.whatsapp.net")),
        is_from_me: false,
        ..Default::default()
    };
    let info = MessageInfo {
        source,
        ..sample_info()
    };
    let event = message_event(
        wa::Message {
            conversation: Some("oi".to_string()),
            ..Default::default()
        },
        info,
    );
    let out = match map_one(&event) {
        Some(PbEvent::Message(m)) => m,
        other => panic!("expected inbound message, got {other:?}"),
    };
    assert_eq!(out.sender, LID_JID);
    assert_eq!(out.sender_alt, SENDER_JID);
    assert_eq!(out.recipient_alt, "5511888000222@s.whatsapp.net");
}

// Absent on the stanza means absent on the wire: the core must not synthesize
// the other namespace from the user part (that would be guessing identity).
#[test]
fn absent_alt_jids_stay_empty_never_synthesized() {
    let out = mapped_inbound(wa::Message {
        conversation: Some("hello".to_string()),
        ..Default::default()
    });
    assert!(out.sender_alt.is_empty(), "got {:?}", out.sender_alt);
    assert!(out.recipient_alt.is_empty(), "got {:?}", out.recipient_alt);
}

#[test]
fn maps_extended_text_with_mentions_and_quote() {
    let out = mapped_inbound(wa::Message {
        extended_text_message: MessageField::some(wa::message::ExtendedTextMessage {
            text: Some("hey @55117770001".to_string()),
            context_info: MessageField::some(wa::ContextInfo {
                mentioned_jid: vec!["55117770001@s.whatsapp.net".to_string()],
                stanza_id: Some("QUOTED-STANZA-1".to_string()),
                participant: Some("55116660002@s.whatsapp.net".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }),
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
        reaction_message: MessageField::some(wa::message::ReactionMessage {
            key: MessageField::some(wa::MessageKey {
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

// E2E triage 2026-06-16: an inbound revoke arrives as an ordinary message
// carrying protocol_message{type:Revoke, key}. The mapper must flag is_delete
// and carry the revoked message's key in protocol_target (the event's own key
// is the revoke stanza id, not the target).
#[test]
fn maps_inbound_revoke_to_is_delete_with_target() {
    let out = mapped_inbound(wa::Message {
        protocol_message: MessageField::some(wa::message::ProtocolMessage {
            r#type: Some(wa::message::protocol_message::Type::REVOKE),
            key: MessageField::some(wa::MessageKey {
                remote_jid: Some(CHAT_JID.to_string()),
                id: Some("REVOKED-MSG-1".to_string()),
                from_me: Some(true),
                participant: Some(SENDER_JID.to_string()),
            }),
            ..Default::default()
        }),
        ..Default::default()
    });
    assert!(out.is_delete);
    assert!(!out.is_edit);
    let target = out
        .protocol_target
        .expect("revoke must carry the target key");
    assert_eq!(target.id, "REVOKED-MSG-1");
    assert_eq!(target.remote_jid, CHAT_JID);
    assert!(target.from_me);
    assert_eq!(target.participant, SENDER_JID);
}

// E2E triage 2026-06-16: a legacy inbound edit carries the new text in
// protocol_message.edited_message; the mapper must flag is_edit, surface that
// new text, and carry the edited message's key in protocol_target.
#[test]
fn maps_inbound_edit_to_is_edit_with_new_text_and_target() {
    let out = mapped_inbound(wa::Message {
        protocol_message: MessageField::some(wa::message::ProtocolMessage {
            r#type: Some(wa::message::protocol_message::Type::MESSAGE_EDIT),
            key: MessageField::some(wa::MessageKey {
                remote_jid: Some(CHAT_JID.to_string()),
                id: Some("EDITED-MSG-1".to_string()),
                from_me: Some(false),
                participant: Some(SENDER_JID.to_string()),
            }),
            edited_message: MessageField::some(wa::Message {
                conversation: Some("edited text".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    });
    assert!(out.is_edit);
    assert!(!out.is_delete);
    assert_eq!(out.text, "edited text");
    let target = out.protocol_target.expect("edit must carry the target key");
    assert_eq!(target.id, "EDITED-MSG-1");
}

// E2E triage re-run 2026-06-16: the REAL inbound edit shape the lib's
// Client::edit_message builds wraps the protocol_message one level deeper, in
// edited_message(FutureProofMessage).message.protocol_message. The unwrapped
// test above missed this exact shape (revoke worked, edit silently didn't); the
// mapper must unwrap edited_message before projecting.
#[test]
fn maps_wrapped_inbound_edit_to_is_edit_with_new_text_and_target() {
    let out = mapped_inbound(wa::Message {
        edited_message: MessageField::some(wa::message::FutureProofMessage {
            message: MessageField::some(wa::Message {
                protocol_message: MessageField::some(wa::message::ProtocolMessage {
                    r#type: Some(wa::message::protocol_message::Type::MESSAGE_EDIT),
                    key: MessageField::some(wa::MessageKey {
                        remote_jid: Some(CHAT_JID.to_string()),
                        id: Some("WRAPPED-EDIT-1".to_string()),
                        from_me: Some(false),
                        participant: Some(SENDER_JID.to_string()),
                    }),
                    edited_message: MessageField::some(wa::Message {
                        conversation: Some("wrapped new text".to_string()),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }),
        ..Default::default()
    });
    assert!(out.is_edit);
    assert!(!out.is_delete);
    assert_eq!(out.text, "wrapped new text");
    let target = out
        .protocol_target
        .expect("wrapped edit must carry the target key");
    assert_eq!(target.id, "WRAPPED-EDIT-1");
}

// The five media-descriptor tests live in event_mapping_media_tests.rs.

#[test]
fn maps_receipt_with_ids_type_and_millis() {
    // 0.7 sealed every event payload (#[non_exhaustive] + a bon builder), so
    // the fixtures below build instead of struct-literal.
    let event = Event::Receipt(
        Receipt::builder()
            .source(sample_source())
            .message_ids(vec!["AAA111".to_string(), "BBB222".to_string()])
            .timestamp(wacore::time::from_secs(1_717_932_111).unwrap())
            .r#type(ReceiptType::Read)
            // 0.7 added this: true only for receipts drained from the offline
            // queue. A live receipt is what this test is about.
            .offline(false)
            .build(),
    );
    match map_one(&event) {
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
    let event = Event::UndecryptableMessage(
        UndecryptableMessage::builder()
            .info(Arc::new(sample_info()))
            .is_unavailable(true)
            .unavailable_type(UnavailableType::ViewOnce)
            .decrypt_fail_mode(DecryptFailMode::Hide)
            .build(),
    );
    match map_one(&event) {
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
    let p = mapped_presence(&Event::Presence(
        PresenceUpdate::builder()
            .from(jid_of(SENDER_JID))
            .unavailable(true)
            .last_seen(wacore::time::from_secs(1_717_932_000).unwrap())
            .build(),
    ));
    assert_eq!(p.jid, SENDER_JID);
    // "unavailable" on the wire means offline for the edge: the flag inverts.
    assert!(!p.online);
    assert_eq!(p.last_seen, 1_717_932_000);
    assert!(p.chat_state.is_empty());
}

#[test]
fn maps_presence_online_without_last_seen() {
    let p = mapped_presence(&Event::Presence(
        PresenceUpdate::builder()
            .from(jid_of(SENDER_JID))
            .unavailable(false)
            .build(),
    ));
    assert!(p.online);
    assert_eq!(p.last_seen, 0);
}

#[test]
fn maps_chat_presence_to_composing_state() {
    let p = mapped_presence(&Event::ChatPresence(
        ChatPresenceUpdate::builder()
            .source(sample_source())
            .state(ChatPresence::Composing)
            .media(ChatPresenceMedia::Text)
            .build(),
    ));
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
    let p = mapped_presence(&Event::ChatPresence(
        ChatPresenceUpdate::builder()
            .source(sample_source())
            .state(ChatPresence::Composing)
            .media(ChatPresenceMedia::Audio)
            .build(),
    ));
    assert_eq!(p.chat_state, "recording");
}

#[test]
fn maps_push_name_update() {
    let event = Event::PushNameUpdate(
        PushNameUpdate::builder()
            .jid(jid_of(SENDER_JID))
            .message(Box::new(sample_info()))
            .old_push_name("Old Alice".to_string())
            .new_push_name("New Alice".to_string())
            .build(),
    );
    match map_one(&event) {
        Some(PbEvent::PushName(p)) => {
            assert_eq!(p.jid, SENDER_JID);
            assert_eq!(p.push_name, "New Alice");
        }
        other => panic!("expected push name, got {other:?}"),
    }
}

#[test]
fn maps_history_sync_with_raw_passthrough() {
    let sync_type = wa::history_sync::HistorySyncType::RECENT;
    let proto = wa::HistorySync {
        sync_type,
        conversations: vec![wa::Conversation {
            id: CHAT_JID.to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let raw = proto.encode_to_vec();
    // 0.7 keeps the chunk zlib-compressed behind the lazy handle, so the fixture
    // has to compress it (and declare the inflated size, which doubles as the
    // inflate cap) the way the real producer does.
    let compressed = zlib_compress(&raw);
    let lazy = LazyHistorySync::new(
        Bytes::from(compressed),
        raw.len(),
        sync_type as i32,
        Some(3),
        Some(75),
    )
    .with_peer_data_request_session_id(Some("sess-1".to_string()));
    match map_one(&Event::HistorySync(Box::new(lazy))) {
        Some(PbEvent::HistorySync(h)) => {
            assert_eq!(h.sync_type, sync_type as i32);
            assert_eq!(h.chunk_order, Some(3));
            assert_eq!(h.progress, Some(75));
            assert_eq!(h.session_id.as_deref(), Some("sess-1"));
            // Verbatim relay: the edge must be able to decode `raw` itself.
            assert_eq!(h.raw, raw);
            let decoded = wa::HistorySync::decode_from_slice(h.raw.as_slice()).unwrap();
            assert_eq!(decoded.sync_type, sync_type);
            assert_eq!(decoded.conversations[0].id, CHAT_JID);
        }
        other => panic!("expected history sync, got {other:?}"),
    }
}

#[test]
fn maps_connected_and_disconnected_states() {
    let connected = mapped_connection(&Event::Connected(Connected::builder().build()));
    assert_eq!(connected.state, pb::ConnectionState::Connected as i32);
    assert!(connected.detail.is_empty());

    // 0.7 turned Disconnected from a unit marker into a payload carrying why
    // the socket closed. The mapping ignores it (the proto has one Disconnected
    // state), so any reason exercises the same arm.
    let disconnected = mapped_connection(&Event::Disconnected(
        Disconnected::builder()
            .reason(wacore::net::DisconnectReason::StreamEnded)
            .build(),
    ));
    assert_eq!(disconnected.state, pb::ConnectionState::Disconnected as i32);
    assert!(disconnected.detail.is_empty());
}

#[test]
fn maps_logged_out_with_reason_in_detail() {
    // 0.7 dropped `MainDeviceGone` from `ConnectFailureReason`; the assertion is
    // about the reason reaching `detail` at all, so any real variant serves.
    let c = mapped_connection(&Event::LoggedOut(
        LoggedOut::builder()
            .on_connect(false)
            .reason(ConnectFailureReason::LoggedOut)
            .build(),
    ));
    assert_eq!(c.state, pb::ConnectionState::LoggedOut as i32);
    assert!(c.detail.contains("LoggedOut"), "detail was {:?}", c.detail);
}

#[test]
fn maps_temporary_ban_to_banned_state() {
    // chrono::Duration without a direct chrono dep: subtract two DateTimes.
    let start = wacore::time::from_secs(0).unwrap();
    let end = wacore::time::from_secs(3_600).unwrap();
    let c = mapped_connection(&Event::TemporaryBan(
        TemporaryBan::builder()
            .code(TempBanReason::BlockedByUsers)
            .expire(end - start)
            .build(),
    ));
    assert_eq!(c.state, pb::ConnectionState::Banned as i32);
    assert!(
        c.detail.contains("BlockedByUsers"),
        "detail was {:?}",
        c.detail
    );
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
    // PairingCodeRefresh has no explicit arm: it must land in Raw, not vanish.
    let event = Event::PairingCodeRefresh(PairingCodeRefresh::builder().force_manual(true).build());
    match map_one(&event) {
        Some(PbEvent::Raw(raw)) => {
            assert_eq!(raw.kind, "PairingCodeRefresh");
            assert_eq!(raw.payload, serde_json::to_vec(&event).unwrap());
            let json: serde_json::Value = serde_json::from_slice(&raw.payload).unwrap();
            assert_eq!(json["PairingCodeRefresh"]["force_manual"], true);
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
            assert_eq!(a.from, SENDER_JID);
            assert_eq!(a.timestamp, 1_717_932_222_000);
            // A plain ack is not a nack: the error field stays empty.
            assert!(a.error.is_empty());
        }
        other => panic!("expected server ack, got {other:?}"),
    }
}

// A nack was only ever a library log line before. Relaying the code is what
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
            // Absent server attrs stay proto3 defaults, never a fake value.
            assert!(a.from.is_empty());
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

fn mapped_app_state(event: &Event) -> pb::AppStateUpdate {
    match map_one(event) {
        Some(PbEvent::AppState(s)) => s,
        other => panic!("expected app-state update, got {other:?}"),
    }
}

#[test]
fn maps_archive_update_to_typed_app_state() {
    let s = mapped_app_state(&Event::ArchiveUpdate(
        ArchiveUpdate::builder()
            .jid(jid_of(CHAT_JID))
            .timestamp(wacore::time::from_secs(1_717_932_000).unwrap())
            .action(Box::default())
            .from_full_sync(false)
            .build(),
    ));
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
    let s = mapped_app_state(&Event::PinUpdate(
        PinUpdate::builder()
            .jid(jid_of(CHAT_JID))
            .timestamp(wacore::time::from_secs(1_717_932_000).unwrap())
            .action(Box::default())
            .from_full_sync(false)
            .build(),
    ));
    assert_eq!(s.chat, CHAT_JID);
    assert_eq!(s.kind, "pin");
}

#[test]
fn maps_mute_update_to_typed_app_state() {
    let s = mapped_app_state(&Event::MuteUpdate(
        MuteUpdate::builder()
            .jid(jid_of(CHAT_JID))
            .timestamp(wacore::time::from_secs(1_717_932_000).unwrap())
            .action(Box::default())
            .from_full_sync(false)
            .build(),
    ));
    assert_eq!(s.chat, CHAT_JID);
    assert_eq!(s.kind, "mute");
}

// StarUpdate names its chat `chat_jid` (it points at a message); the mapping
// must read that field, not the missing `jid`.
#[test]
fn maps_star_update_reads_chat_jid() {
    let s = mapped_app_state(&Event::StarUpdate(
        StarUpdate::builder()
            .chat_jid(jid_of(CHAT_JID))
            .message_id("WAMID-STAR".to_string())
            .from_me(false)
            .timestamp(wacore::time::from_secs(1_717_932_000).unwrap())
            .action(Box::default())
            .from_full_sync(false)
            .build(),
    ));
    assert_eq!(s.chat, CHAT_JID);
    assert_eq!(s.kind, "star");
}

#[test]
fn maps_mark_chat_as_read_update_to_typed_app_state() {
    let s = mapped_app_state(&Event::MarkChatAsReadUpdate(
        MarkChatAsReadUpdate::builder()
            .jid(jid_of(CHAT_JID))
            .timestamp(wacore::time::from_secs(1_717_932_000).unwrap())
            .action(Box::default())
            .from_full_sync(false)
            .build(),
    ));
    assert_eq!(s.chat, CHAT_JID);
    assert_eq!(s.kind, "mark_read");
}

#[test]
fn maps_delete_chat_update_to_typed_app_state() {
    let s = mapped_app_state(&Event::DeleteChatUpdate(
        DeleteChatUpdate::builder()
            .jid(jid_of(CHAT_JID))
            .delete_media(false)
            .timestamp(wacore::time::from_secs(1_717_932_000).unwrap())
            .action(Box::default())
            .from_full_sync(false)
            .build(),
    ));
    assert_eq!(s.chat, CHAT_JID);
    assert_eq!(s.kind, "delete_chat");
}

#[test]
fn maps_incoming_call_offer_to_typed_call_event() {
    let event = Event::IncomingCall(
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
    );
    match map_one(&event) {
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
    let event = Event::IncomingCall(
        IncomingCall::builder()
            .from(jid_of(SENDER_JID))
            .stanza_id("STANZA-CALL-2".to_string())
            .timestamp(wacore::time::from_secs(1_717_932_000).unwrap())
            .offline(false)
            .action(CallAction::Terminate {
                call_id: "CALL-ID-2".to_string(),
                call_creator: jid_of(SENDER_JID),
                // New in 0.7: why the peer hung up. Absent = a plain missed call.
                reason: None,
                duration: Some(42),
                audio_duration: None,
            })
            .build(),
    );
    match map_one(&event) {
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
