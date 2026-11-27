//! A message inside a wrapper projects like the bare message (#281): text,
//! media and edit/revoke are read from the same base message the library
//! finds the mentions and the quote in, so one message never projects half.
//! `raw_message` keeps the payload whole, wrappers included.

use super::*;
use whatsapp_rust::buffa::MessageField;

const CHAT: &str = "120363041234567281@g.us";
const MENTIONED: &str = "55117770281@s.whatsapp.net";

/// The projection every inbound and echoed message goes through.
fn projected(msg: &wa::Message) -> pb::InboundMessage {
    map_sent(
        pb::MessageKey::default(),
        CHAT,
        "5511888888888@s.whatsapp.net",
        0,
        msg,
    )
}

/// `inner` inside a `FutureProofMessage`, the shape of every classified wrapper.
fn future_proof(inner: wa::Message) -> MessageField<wa::message::FutureProofMessage> {
    let mut wrapper = wa::message::FutureProofMessage::default();
    wrapper.message = MessageField::some(inner);
    MessageField::some(wrapper)
}

/// An extended text that mentions `MENTIONED`.
fn text_mentioning(text: &str) -> wa::Message {
    let mut context = wa::ContextInfo::default();
    context.mentioned_jid = vec![MENTIONED.to_string()];
    let mut ext = wa::message::ExtendedTextMessage::default();
    ext.text = Some(text.to_string());
    ext.context_info = MessageField::some(context);
    let mut msg = wa::Message::default();
    msg.extended_text_message = MessageField::some(ext);
    msg
}

fn spoiler(inner: wa::Message) -> wa::Message {
    let mut msg = wa::Message::default();
    msg.spoiler_message = future_proof(inner);
    msg
}

#[test]
fn spoiler_wrapped_text_projects_its_text_and_mentions() {
    let out = projected(&spoiler(text_mentioning("hidden @55117770281")));
    assert_eq!(out.text, "hidden @55117770281");
    let mentioned: Vec<_> = out.mentions.iter().map(|m| m.jid.clone()).collect();
    assert_eq!(mentioned, [relay_jid(MENTIONED)]);
}

#[test]
fn group_mentioned_text_projects_its_text() {
    let mut msg = wa::Message::default();
    msg.group_mentioned_message = future_proof(text_mentioning("everyone, look"));
    assert_eq!(projected(&msg).text, "everyone, look");
}

#[test]
fn ephemeral_wrapped_image_projects_its_media_and_caption() {
    let mut image = wa::message::ImageMessage::default();
    image.mimetype = Some("image/jpeg".to_string());
    image.direct_path = Some("/v/t62.ephemeral".to_string());
    image.caption = Some("gone in a day".to_string());
    let mut inner = wa::Message::default();
    inner.image_message = MessageField::some(image);
    let mut msg = wa::Message::default();
    msg.ephemeral_message = future_proof(inner);
    let out = projected(&msg);
    let media = out.media.expect("the wrapped image's descriptor");
    assert_eq!(media.media_type(), pb::MediaType::Image);
    assert_eq!(media.direct_path, "/v/t62.ephemeral");
    assert_eq!(out.caption, "gone in a day");
}

/// The sound itself is #278; here the sticker still reaches the edge as the
/// plain sticker it is underneath.
#[test]
fn audio_sticker_projects_the_sticker_media() {
    let mut sticker = wa::message::StickerMessage::default();
    sticker.mimetype = Some("image/webp".to_string());
    sticker.direct_path = Some("/v/t62.sticker".to_string());
    let mut inner = wa::Message::default();
    inner.sticker_message = MessageField::some(sticker);
    let mut msg = wa::Message::default();
    msg.audio_sticker_message = future_proof(inner);
    let media = projected(&msg).media.expect("the sticker's descriptor");
    assert_eq!(media.media_type(), pb::MediaType::Sticker);
    assert_eq!(media.direct_path, "/v/t62.sticker");
}

#[test]
fn a_revoke_inside_ephemeral_is_a_delete_of_its_target() {
    use wa::message::protocol_message::Type;
    let mut key = wa::MessageKey::default();
    key.remote_jid = Some(CHAT.to_string());
    key.id = Some("3EB0GONE281".to_string());
    let mut revoke = wa::message::ProtocolMessage::default();
    revoke.key = MessageField::some(key);
    revoke.r#type = Some(Type::REVOKE);
    let mut inner = wa::Message::default();
    inner.protocol_message = MessageField::some(revoke);
    let mut msg = wa::Message::default();
    msg.ephemeral_message = future_proof(inner);
    let out = projected(&msg);
    assert!(out.is_delete, "a revoke inside the ephemeral wrapper");
    assert_eq!(out.protocol_target.expect("target").id, "3EB0GONE281");
}

#[test]
fn raw_message_keeps_the_wrappers() {
    let msg = spoiler(text_mentioning("hidden"));
    let out = projected(&msg);
    assert_eq!(out.raw_message, msg.encode_to_vec());
    let decoded = wa::Message::decode_from_slice(&out.raw_message).expect("decode");
    assert!(
        decoded.spoiler_message.is_set(),
        "the wrapper stays in raw_message"
    );
}
