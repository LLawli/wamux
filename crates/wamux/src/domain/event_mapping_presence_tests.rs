//! Receipts, undecryptable messages, presence, chat states and history sync.

use super::*;
use std::sync::Arc;
use whatsapp_rust::waproto::whatsapp as wa;

use bytes::Bytes;
use wacore::types::events::{
    ChatPresenceUpdate, DecryptFailMode, LazyHistorySync, PresenceUpdate, Receipt, UnavailableType,
    UndecryptableMessage,
};
use wacore::types::message::MessageSource;
use wacore::types::presence::{ChatActivity, PresenceStatus, ReceiptType};

use crate::proto::v1::event_envelope::Event as PbEvent;

use super::tests::{
    CHAT_JID, SENDER_JID, jid_of, map_one, mapped_presence, sample_info, sample_source, wire,
    zlib_compress,
};

#[test]
fn maps_receipt_with_ids_type_and_millis() {
    // Event payloads are #[non_exhaustive] with a bon builder, so the fixtures
    // below build instead of struct-literal.
    let event = Event::Receipt(
        Receipt::builder()
            .source(sample_source())
            .message_ids(vec!["AAA111".into(), "BBB222".into()])
            .timestamp(wacore::time::from_secs(1_717_932_111).unwrap())
            .r#type(ReceiptType::Read)
            // True only for receipts drained from the offline queue. A live
            // receipt is what this test is about.
            .offline(false)
            .build(),
    );
    match map_one(&event) {
        Some(PbEvent::Receipt(r)) => {
            assert_eq!(r.chat, wire(CHAT_JID));
            assert_eq!(r.sender, wire(SENDER_JID));
            assert_eq!(r.message_ids, ["AAA111", "BBB222"]);
            // #126: an enum, never Debug casing or a free token; no raw when named.
            assert_eq!(r.r#type(), pb::ReceiptType::Read);
            assert_eq!(r.type_raw, "");
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
            assert_eq!(u.chat, wire(CHAT_JID));
            assert_eq!(u.sender, wire(SENDER_JID));
            // #126: an enum, not the `Debug` string "ViewOnce".
            assert_eq!(u.reason(), pb::UnavailableReason::ViewOnce);
        }
        other => panic!("expected undecryptable, got {other:?}"),
    }
}

#[test]
fn maps_presence_offline_with_last_seen_seconds() {
    let p = mapped_presence(&Event::Presence(
        PresenceUpdate::builder()
            .from(jid_of(SENDER_JID))
            .status(PresenceStatus::Unavailable)
            .last_seen(wacore::time::from_secs(1_717_932_000).unwrap())
            .build(),
    ));
    assert_eq!(p.jid, wire(SENDER_JID));
    // "unavailable" on the wire means offline for the edge: the flag inverts.
    assert_eq!(p.online, Some(false));
    assert_eq!(p.last_seen, 1_717_932_000);
    // #126: no chat state on real presence is UNSPECIFIED, not a token.
    assert_eq!(p.chat_state(), pb::ChatState::Unspecified);
    // Real presence is not scoped to a conversation (issue #24): unset (#122).
    assert_eq!(p.chat, None);
}

#[test]
fn maps_presence_online_without_last_seen() {
    let p = mapped_presence(&Event::Presence(
        PresenceUpdate::builder()
            .from(jid_of(SENDER_JID))
            .status(PresenceStatus::Available)
            .build(),
    ));
    assert_eq!(p.online, Some(true));
    assert_eq!(p.last_seen, 0);
}

#[test]
fn maps_chat_presence_to_composing_state() {
    let p = mapped_presence(&Event::ChatPresence(
        ChatPresenceUpdate::builder()
            .source(sample_source())
            .state(ChatActivity::Typing)
            .build(),
    ));
    assert_eq!(p.jid, wire(SENDER_JID));
    // A chat state measures no presence: a hardcoded true would light an
    // "online" dot off a literal (issue #24).
    assert_eq!(p.online, None);
    // #126: the ChatState enum (not the token "composing").
    assert_eq!(p.chat_state(), pb::ChatState::Composing);
}

// Regression (issue #24): the conversation a chat state happened in must be
// named, or typing in a group and typing in the DM with the same person arrive
// as identical bytes and the edge draws the indicator on the DM.
#[test]
fn chat_presence_in_a_group_names_the_group_not_only_the_sender() {
    let p = mapped_presence(&Event::ChatPresence(
        ChatPresenceUpdate::builder()
            .source(sample_source())
            .state(ChatActivity::Typing)
            .build(),
    ));
    assert_eq!(p.jid, wire(SENDER_JID));
    assert_eq!(p.chat, wire(CHAT_JID));
}

#[test]
fn chat_presence_in_a_direct_chat_names_the_contact_as_the_conversation() {
    let source = MessageSource {
        chat: jid_of(SENDER_JID),
        sender: jid_of(SENDER_JID),
        ..Default::default()
    };
    let p = mapped_presence(&Event::ChatPresence(
        ChatPresenceUpdate::builder()
            .source(source)
            .state(ChatActivity::Idle)
            .build(),
    ));
    assert_eq!(p.chat, wire(SENDER_JID));
    assert_eq!(p.chat_state(), pb::ChatState::Paused);
}

// The lib models recording as `ChatActivity::RecordingAudio` (Composing with
// media=Audio on the wire). The wire contract flattens it back to "recording".
#[test]
fn maps_chat_presence_audio_to_recording_state() {
    let p = mapped_presence(&Event::ChatPresence(
        ChatPresenceUpdate::builder()
            .source(sample_source())
            .state(ChatActivity::RecordingAudio)
            .build(),
    ));
    assert_eq!(p.chat_state(), pb::ChatState::Recording);
}

// No `maps_push_name_update`: the library retired `Event::PushNameUpdate`
// (#30, upstream #1310) as a dead event. `pb::PushNameUpdate` stays in the
// proto so the contract does not break; nothing emits it.

#[test]
fn maps_history_sync_with_raw_passthrough() {
    let sync_type = wa::history_sync::HistorySyncType::RECENT;
    let proto = {
        let mut proto = wa::HistorySync::default();
        proto.sync_type = sync_type;
        proto.conversations = vec![{
            let mut proto = wa::Conversation::default();
            proto.id = CHAT_JID.to_string();
            proto
        }];
        proto
    };
    let raw = proto.encode_to_vec();
    // The library keeps the chunk zlib-compressed behind the lazy handle, so
    // the fixture has to compress it (and declare the inflated size, which
    // doubles as the inflate cap) the way the real producer does.
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
