use super::*;
use std::sync::Arc;
use wacore::types::message::MessageInfo;
use whatsapp_rust::waproto::whatsapp as wa;

use wacore::types::events::{BatchOrigin, InboundMessage, MessageBatch};
use wacore::types::message::MessageSource;
use whatsapp_rust::Jid;
use whatsapp_rust::buffa::MessageField;

use crate::proto::v1::event_envelope::Event as PbEvent;

pub(super) const CHAT_JID: &str = "120363041234567890@g.us";

/// A jid as the core relays it out (#120): the value verbatim.
pub(super) fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}
pub(super) const SENDER_JID: &str = "5511999000111@s.whatsapp.net";
pub(super) const LID_JID: &str = "169815004184633@lid";

/// Minimal valid WABinary node `<iq/>`: LIST_8 list of size 1, then the tag
/// as a BINARY_8 string. Hand-rolled because whatsapp-rust's marshal helpers
/// are `#[cfg(test)]`-gated and wacore-binary is not a default wamux dep.
pub(super) const RAW_IQ_NODE: &[u8] = &[0xF8, 0x01, 0xFC, 0x02, b'i', b'q'];

/// zlib-compress a history-sync payload the way the server does, so the lazy
/// handle can inflate it back. `flate2` is already in the tree (via the lib).
pub(super) fn zlib_compress(raw: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(raw).expect("in-memory write cannot fail");
    encoder.finish().expect("in-memory finish cannot fail")
}

pub(super) fn jid_of(value: &str) -> Jid {
    value.parse().unwrap()
}

pub(super) fn sample_source() -> MessageSource {
    MessageSource {
        chat: jid_of(CHAT_JID),
        sender: jid_of(SENDER_JID),
        is_from_me: false,
        ..Default::default()
    }
}

pub(super) fn sample_info() -> MessageInfo {
    MessageInfo {
        source: sample_source(),
        // A CompactString in the library (#30).
        id: "WAMID-1".into(),
        push_name: "Alice".into(),
        // Fixed non-epoch instant: epoch (the Default) would hide a broken
        // timestamp mapping behind a zero.
        timestamp: wacore::time::from_secs(1_717_932_000).unwrap(),
        ..Default::default()
    }
}

/// The library delivers inbound messages in batches, so `map_event` returns a list.
/// Every event these tests build maps to exactly one payload; asserting that
/// here keeps each test's own assertion about the payload, not about arity.
pub(super) fn map_one(event: &Event) -> Option<PbEvent> {
    let mut mapped = map_event(event);
    assert!(
        mapped.len() <= 1,
        "this helper is for single-payload events; got {}",
        mapped.len()
    );
    mapped.pop()
}

/// One inbound message wrapped in the batch-of-one the lib dispatches for live
/// traffic. Both payload structs are `#[non_exhaustive]`, hence the builders.
pub(super) fn message_event(msg: wa::Message, info: MessageInfo) -> Event {
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

pub(super) fn mapped_inbound(msg: wa::Message) -> pb::InboundMessage {
    match map_one(&message_event(msg, sample_info())) {
        Some(PbEvent::Message(m)) => m,
        other => panic!("expected inbound message, got {other:?}"),
    }
}

/// pub(super): the media-descriptor suite (event_mapping_media_tests.rs) uses it.
pub(super) fn mapped_media(msg: wa::Message) -> (pb::MediaDescriptor, String) {
    let out = mapped_inbound(msg);
    let media = out
        .media
        .expect("media sub-message must yield a descriptor");
    (media, out.caption)
}

pub(super) fn sent_key(chat: &str, id: &str) -> pb::MessageKey {
    pb::MessageKey {
        chat: wire(chat),
        id: id.to_string(),
        from_me: true,
        participant: None,
    }
}

pub(super) fn mapped_connection(event: &Event) -> pb::ConnectionStateChanged {
    match map_one(event) {
        Some(PbEvent::Connection(c)) => c,
        other => panic!("expected connection event, got {other:?}"),
    }
}

pub(super) fn mapped_presence(event: &Event) -> pb::PresenceUpdate {
    match map_one(event) {
        Some(PbEvent::Presence(p)) => p,
        other => panic!("expected presence, got {other:?}"),
    }
}

#[test]
fn maps_conversation_text_message_with_key_and_metadata() {
    let out = mapped_inbound({
        let mut proto = wa::Message::default();
        proto.conversation = Some("hello world".to_string());
        proto
    });
    assert_eq!(out.text, "hello world");
    assert_eq!(out.chat, wire(CHAT_JID));
    assert_eq!(out.sender, wire(SENDER_JID));
    assert_eq!(out.push_name, "Alice");
    assert_eq!(out.timestamp, 1_717_932_000_000);

    let key = out.key.expect("inbound message must carry a key");
    assert_eq!(key.chat, wire(CHAT_JID));
    assert_eq!(key.id, "WAMID-1");
    assert!(!key.from_me);
    assert_eq!(key.participant, wire(SENDER_JID));

    // raw_message is the full encoded wa.Message; decoding restores the text.
    assert!(!out.raw_message.is_empty());
    let decoded = wa::Message::decode_from_slice(out.raw_message.as_slice()).unwrap();
    assert_eq!(decoded.conversation.as_deref(), Some("hello world"));
}

// REGRESSION (issue #1): a `@lid` sender is unidentifiable on its own, and the
// stanza already carries the phone side in `sender_alt`. Dropping it forces the
// edge to poll for an identity the event was holding.
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
        {
            let mut proto = wa::Message::default();
            proto.conversation = Some("oi".to_string());
            proto
        },
        info,
    );
    let out = match map_one(&event) {
        Some(PbEvent::Message(m)) => m,
        other => panic!("expected inbound message, got {other:?}"),
    };
    assert_eq!(out.sender, wire(LID_JID));
    assert_eq!(out.sender_alt, wire(SENDER_JID));
    assert_eq!(out.recipient_alt, wire("5511888000222@s.whatsapp.net"));
}

// Absent on the stanza means absent on the wire: the core must not synthesize
// the other namespace from the user part (that would be guessing identity).
// #122: absent is an unset Jid, never `Jid { value: "" }`.
#[test]
fn absent_alt_jids_stay_unset_never_synthesized() {
    let out = mapped_inbound({
        let mut proto = wa::Message::default();
        proto.conversation = Some("hello".to_string());
        proto
    });
    assert_eq!(out.sender_alt, None);
    assert_eq!(out.recipient_alt, None);
}

#[test]
fn maps_extended_text_with_mentions_and_quote() {
    let out = mapped_inbound({
        let mut proto = wa::Message::default();
        proto.extended_text_message = MessageField::some({
            let mut proto = wa::message::ExtendedTextMessage::default();
            proto.text = Some("hey @55117770001".to_string());
            proto.context_info = MessageField::some({
                let mut proto = wa::ContextInfo::default();
                proto.mentioned_jid = vec!["55117770001@s.whatsapp.net".to_string()];
                proto.stanza_id = Some("QUOTED-STANZA-1".to_string());
                proto.participant = Some("55116660002@s.whatsapp.net".to_string());
                proto
            });
            proto
        });
        proto
    });
    assert_eq!(out.text, "hey @55117770001");
    assert_eq!(
        out.mentions,
        [pb::Mention {
            jid: wire("55117770001@s.whatsapp.net"),
        }]
    );
    let quote = out.quote.expect("stanza_id must produce a quote");
    assert_eq!(quote.participant, wire("55116660002@s.whatsapp.net"));
    let quoted = quote.quoted.expect("quote must carry the quoted key");
    assert_eq!(quoted.id, "QUOTED-STANZA-1");
    assert_eq!(quoted.chat, wire(CHAT_JID));
    assert_eq!(quoted.participant, wire("55116660002@s.whatsapp.net"));
    assert!(!quoted.from_me);
}

#[test]
fn maps_reaction_message_with_target_key() {
    let out = mapped_inbound({
        let mut proto = wa::Message::default();
        proto.reaction_message = MessageField::some({
            let mut proto = wa::message::ReactionMessage::default();
            proto.key = MessageField::some({
                let mut proto = wa::MessageKey::default();
                proto.remote_jid = Some(CHAT_JID.to_string());
                proto.id = Some("TARGET-MSG-1".to_string());
                proto.from_me = Some(true);
                proto.participant = Some(SENDER_JID.to_string());
                proto
            });
            // Reaction payloads are emoji on the wire by protocol.
            proto.text = Some("\u{1F44D}".to_string());
            proto
        });
        proto
    });
    assert_eq!(out.reaction, "\u{1F44D}");
    let target = out
        .reaction_target
        .expect("reaction key must map to a target");
    assert_eq!(target.chat, wire(CHAT_JID));
    assert_eq!(target.id, "TARGET-MSG-1");
    assert!(target.from_me);
    assert_eq!(target.participant, wire(SENDER_JID));
}

// An inbound revoke arrives as an ordinary message carrying
// protocol_message{type:Revoke, key}. The mapper must flag is_delete and carry
// the revoked message's key in protocol_target (the event's own key is the
// revoke stanza id, not the target).
#[test]
fn maps_inbound_revoke_to_is_delete_with_target() {
    let out = mapped_inbound({
        let mut proto = wa::Message::default();
        proto.protocol_message = MessageField::some({
            let mut proto = wa::message::ProtocolMessage::default();
            proto.r#type = Some(wa::message::protocol_message::Type::REVOKE);
            proto.key = MessageField::some({
                let mut proto = wa::MessageKey::default();
                proto.remote_jid = Some(CHAT_JID.to_string());
                proto.id = Some("REVOKED-MSG-1".to_string());
                proto.from_me = Some(true);
                proto.participant = Some(SENDER_JID.to_string());
                proto
            });
            proto
        });
        proto
    });
    assert!(out.is_delete);
    assert!(!out.is_edit);
    let target = out
        .protocol_target
        .expect("revoke must carry the target key");
    assert_eq!(target.id, "REVOKED-MSG-1");
    assert_eq!(target.chat, wire(CHAT_JID));
    assert!(target.from_me);
    assert_eq!(target.participant, wire(SENDER_JID));
}

// A legacy inbound edit carries the new text in
// protocol_message.edited_message; the mapper must flag is_edit, surface that
// new text, and carry the edited message's key in protocol_target.
#[test]
fn maps_inbound_edit_to_is_edit_with_new_text_and_target() {
    let out = mapped_inbound({
        let mut proto = wa::Message::default();
        proto.protocol_message = MessageField::some({
            let mut proto = wa::message::ProtocolMessage::default();
            proto.r#type = Some(wa::message::protocol_message::Type::MESSAGE_EDIT);
            proto.key = MessageField::some({
                let mut proto = wa::MessageKey::default();
                proto.remote_jid = Some(CHAT_JID.to_string());
                proto.id = Some("EDITED-MSG-1".to_string());
                proto.from_me = Some(false);
                proto.participant = Some(SENDER_JID.to_string());
                proto
            });
            proto.edited_message = MessageField::some({
                let mut proto = wa::Message::default();
                proto.conversation = Some("edited text".to_string());
                proto
            });
            proto
        });
        proto
    });
    assert!(out.is_edit);
    assert!(!out.is_delete);
    assert_eq!(out.text, "edited text");
    let target = out.protocol_target.expect("edit must carry the target key");
    assert_eq!(target.id, "EDITED-MSG-1");
}

// The REAL inbound edit shape the lib's Client::edit_message builds wraps the
// protocol_message one level deeper, in
// edited_message(FutureProofMessage).message.protocol_message. The unwrapped
// test above does not cover this shape, and a mapper that reads only the top
// level still sees revokes but silently drops every edit; the mapper must
// unwrap edited_message before projecting.
#[test]
fn maps_wrapped_inbound_edit_to_is_edit_with_new_text_and_target() {
    let out = mapped_inbound({
        let mut proto = wa::Message::default();
        proto.edited_message = MessageField::some({
            let mut proto = wa::message::FutureProofMessage::default();
            proto.message = MessageField::some({
                let mut proto = wa::Message::default();
                proto.protocol_message = MessageField::some({
                    let mut proto = wa::message::ProtocolMessage::default();
                    proto.r#type = Some(wa::message::protocol_message::Type::MESSAGE_EDIT);
                    proto.key = MessageField::some({
                        let mut proto = wa::MessageKey::default();
                        proto.remote_jid = Some(CHAT_JID.to_string());
                        proto.id = Some("WRAPPED-EDIT-1".to_string());
                        proto.from_me = Some(false);
                        proto.participant = Some(SENDER_JID.to_string());
                        proto
                    });
                    proto.edited_message = MessageField::some({
                        let mut proto = wa::Message::default();
                        proto.conversation = Some("wrapped new text".to_string());
                        proto
                    });
                    proto
                });
                proto
            });
            proto
        });
        proto
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
