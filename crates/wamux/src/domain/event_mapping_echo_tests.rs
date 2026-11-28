//! The echo of a message this relay sent, projected like an inbound one (#22).

use super::*;
use whatsapp_rust::waproto::whatsapp as wa;

use whatsapp_rust::buffa::MessageField;

use super::tests::{sent_key, wire};

/// Issue #22: the echo of a message this relay sent must come out of the SAME
/// projection an inbound message does, or an edge ends up with two shapes for
/// one concept.
#[test]
fn a_sent_text_echoes_with_the_inbound_shape() {
    let msg = {
        let mut proto = wa::Message::default();
        proto.conversation = Some("oi".to_string());
        proto
    };
    let out = crate::domain::event_mapping::map_sent(
        pb::MessageKey {
            chat: wire("5511999999999@s.whatsapp.net"),
            id: "3EB0SENT".to_string(),
            from_me: true,
            participant: None,
        },
        "5511999999999@s.whatsapp.net",
        "5511888888888@s.whatsapp.net",
        1_756_800_000_000,
        &msg,
    );
    let key = out.key.expect("key must be set");
    assert!(key.from_me, "an echo is always from_me");
    assert_eq!(key.id, "3EB0SENT");
    // #122: both ends are Jids, verbatim.
    assert_eq!(out.chat, wire("5511999999999@s.whatsapp.net"));
    assert_eq!(out.sender, wire("5511888888888@s.whatsapp.net"));
    assert_eq!(out.text, "oi");
    assert_eq!(out.timestamp, 1_756_800_000_000);
}

/// The bytes that went out, not a reconstruction: an edge that decodes
/// raw_message (polls, stickers) must get the real payload.
#[test]
fn an_echo_carries_the_bytes_that_went_out() {
    let msg = {
        let mut proto = wa::Message::default();
        proto.conversation = Some("oi".to_string());
        proto
    };
    let out = crate::domain::event_mapping::map_sent(
        pb::MessageKey::default(),
        "5511999999999@s.whatsapp.net",
        "5511888888888@s.whatsapp.net",
        0,
        &msg,
    );
    assert_eq!(out.raw_message, msg.encode_to_vec());
}

/// Media echoes through the same descriptor extraction, so the edge downloads
/// its own send exactly like anyone else's.
#[test]
fn a_sent_image_echoes_its_descriptor_and_caption() {
    let msg = {
        let mut proto = wa::Message::default();
        proto.image_message = whatsapp_rust::buffa::MessageField::some({
            let mut proto = wa::message::ImageMessage::default();
            proto.mimetype = Some("image/jpeg".to_string());
            proto.direct_path = Some("/v/t62.sent".to_string());
            proto.file_length = Some(4096);
            proto.caption = Some("a cat".to_string());
            proto
        });
        proto
    };
    let out = crate::domain::event_mapping::map_sent(
        pb::MessageKey::default(),
        "5511999999999@s.whatsapp.net",
        "5511888888888@s.whatsapp.net",
        0,
        &msg,
    );
    let media = out.media.expect("a sent image must carry its descriptor");
    assert_eq!(media.media_type(), pb::MediaType::Image);
    assert_eq!(media.direct_path, "/v/t62.sent");
    assert_eq!(out.caption, "a cat");
}

/// Issue #38: an edit is built inside the library (upstream `build_edit_message`,
/// a top-level MESSAGE_EDIT protocol message) and handed back on
/// `SendResult::message`. Its echo must read as an edit of the target, with the
/// new text, exactly like the edit another device of the account would see.
#[test]
fn a_sent_edit_echoes_as_an_edit_of_its_target() {
    use wa::message::protocol_message::Type;
    let chat = "5511999999999@s.whatsapp.net";
    let msg = {
        let mut proto = wa::Message::default();
        proto.protocol_message = MessageField::some({
            let mut proto = wa::message::ProtocolMessage::default();
            proto.key = MessageField::some({
                let mut proto = wa::MessageKey::default();
                proto.remote_jid = Some(chat.to_string());
                proto.from_me = Some(true);
                proto.id = Some("3EB0ORIGINAL".to_string());
                proto.participant = None;
                proto
            });
            proto.r#type = Some(Type::MESSAGE_EDIT);
            proto.edited_message = MessageField::some({
                let mut proto = wa::Message::default();
                proto.conversation = Some("fixed typo".to_string());
                proto
            });
            proto.timestamp_ms = Some(1_756_800_000_000);
            proto
        });
        proto
    };
    let out = crate::domain::event_mapping::map_sent(
        sent_key(chat, "3EB0EDITSTANZA"),
        chat,
        "5511888888888@s.whatsapp.net",
        0,
        &msg,
    );
    assert!(out.is_edit);
    assert_eq!(out.text, "fixed typo");
    let target = out.protocol_target.expect("an edit names its target");
    assert_eq!(target.id, "3EB0ORIGINAL");
    // #120: the wa key's absent participant (a DM) and its chat relay as they
    // are: an unset Jid, never `Jid { value: "" }`, and the chat verbatim.
    assert_eq!(target.chat, wire(chat));
    assert_eq!(target.participant, None);
    // The echo's own key is the edit stanza, never the message it edits.
    assert_eq!(out.key.expect("key").id, "3EB0EDITSTANZA");
}

/// Issue #38: same for a revoke (upstream `build_revoke_message`).
#[test]
fn a_sent_revoke_echoes_as_a_delete_of_its_target() {
    use wa::message::protocol_message::Type;
    let chat = "120363041234567890@g.us";
    let msg = {
        let mut proto = wa::Message::default();
        proto.protocol_message = MessageField::some({
            let mut proto = wa::message::ProtocolMessage::default();
            proto.key = MessageField::some({
                let mut proto = wa::MessageKey::default();
                proto.remote_jid = Some(chat.to_string());
                proto.from_me = Some(true);
                proto.id = Some("3EB0GONE".to_string());
                proto.participant = None;
                proto
            });
            proto.r#type = Some(Type::REVOKE);
            proto
        });
        proto
    };
    let out = crate::domain::event_mapping::map_sent(
        sent_key(chat, "3EB0REVOKESTANZA"),
        chat,
        "5511888888888@s.whatsapp.net",
        0,
        &msg,
    );
    assert!(out.is_delete);
    assert!(!out.is_edit);
    assert_eq!(out.protocol_target.expect("target").id, "3EB0GONE");
    assert_eq!(out.key.expect("key").id, "3EB0REVOKESTANZA");
}

/// Issue #41: a status revoke is the library's `status().revoke`, a REVOKE
/// protocol message keyed on `status@broadcast`. It echoes as a delete of the
/// status, in the status chat, like the chat revoke above.
#[test]
fn a_sent_status_revoke_echoes_as_a_delete_of_the_status() {
    use wa::message::protocol_message::Type;
    let chat = "status@broadcast";
    let msg = {
        let mut proto = wa::Message::default();
        proto.protocol_message = MessageField::some({
            let mut proto = wa::message::ProtocolMessage::default();
            proto.key = MessageField::some({
                let mut proto = wa::MessageKey::default();
                proto.remote_jid = Some(chat.to_string());
                proto.from_me = Some(true);
                proto.id = Some("3EB0STATUS".to_string());
                proto.participant = None;
                proto
            });
            proto.r#type = Some(Type::REVOKE);
            proto
        });
        proto
    };
    let out = crate::domain::event_mapping::map_sent(
        sent_key(chat, "3EB0STATUSREVOKE"),
        chat,
        "5511888888888@s.whatsapp.net",
        0,
        &msg,
    );
    assert!(out.is_delete);
    let target = out
        .protocol_target
        .expect("a status revoke names the status");
    assert_eq!(target.id, "3EB0STATUS");
    assert_eq!(target.chat, wire(chat));
    let key = out.key.expect("key");
    assert_eq!(key.id, "3EB0STATUSREVOKE");
    assert_eq!(key.chat, wire(chat));
}
