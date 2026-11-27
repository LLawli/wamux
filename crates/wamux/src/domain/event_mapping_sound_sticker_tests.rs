//! A received sound sticker (#278): `media` stays the sticker image and
//! `sticker_audio` describes the clip, which DownloadMedia fetches as audio.

use super::*;
use whatsapp_rust::buffa::MessageField;
use whatsapp_rust::waproto::whatsapp::message::sticker_message::Audio;

const CHAT: &str = "5511888888888@s.whatsapp.net";

/// The projection every inbound and echoed message goes through.
fn projected(msg: &wa::Message) -> pb::InboundMessage {
    map_sent(pb::MessageKey::default(), CHAT, CHAT, 0, msg)
}

fn clip() -> wa::message::AudioMessage {
    let mut clip = wa::message::AudioMessage::default();
    clip.mimetype = Some("audio/ogg; codecs=opus".to_string());
    clip.direct_path = Some("/v/t62.7114-24/clip".to_string());
    clip.media_key = Some(vec![4; 32]);
    clip.file_enc_sha256 = Some(vec![5; 32]);
    clip.file_sha256 = Some(vec![6; 32]);
    clip.file_length = Some(9_000);
    clip.seconds = Some(3);
    clip
}

fn sticker(with_clip: bool) -> wa::message::StickerMessage {
    let mut sticker = wa::message::StickerMessage::default();
    sticker.mimetype = Some("image/webp".to_string());
    sticker.direct_path = Some("/v/t62.15575-24/sticker".to_string());
    sticker.media_key = Some(vec![1; 32]);
    sticker.file_length = Some(20_000);
    if with_clip {
        sticker.audio = Some(Audio::AudioMessage(Box::new(clip())));
    }
    sticker
}

/// The shape the apps send: the sticker inside `audioStickerMessage`.
fn wrapped_sound_sticker() -> wa::Message {
    let mut inner = wa::Message::default();
    inner.sticker_message = MessageField::some(sticker(true));
    let mut wrapper = wa::message::FutureProofMessage::default();
    wrapper.message = MessageField::some(inner);
    let mut msg = wa::Message::default();
    msg.audio_sticker_message = MessageField::some(wrapper);
    msg
}

fn assert_is_the_clip(descriptor: &pb::MediaDescriptor) {
    assert_eq!(descriptor.media_type(), pb::MediaType::Audio);
    assert_eq!(descriptor.direct_path, "/v/t62.7114-24/clip");
    assert_eq!(descriptor.media_key, vec![4; 32]);
    assert_eq!(descriptor.file_enc_sha256, vec![5; 32]);
    assert_eq!(descriptor.file_sha256, vec![6; 32]);
    assert_eq!(descriptor.file_length, 9_000);
    assert_eq!(descriptor.mime_type, "audio/ogg; codecs=opus");
}

#[test]
fn a_sound_sticker_projects_its_clip() {
    let out = projected(&wrapped_sound_sticker());
    let media = out.media.expect("the sticker image");
    assert_eq!(media.media_type(), pb::MediaType::Sticker);
    assert_eq!(media.direct_path, "/v/t62.15575-24/sticker");
    assert_is_the_clip(&out.sticker_audio.expect("the clip"));
}

#[test]
fn a_bare_sticker_with_audio_projects_its_clip() {
    let mut msg = wa::Message::default();
    msg.sticker_message = MessageField::some(sticker(true));
    let out = projected(&msg);
    assert_eq!(
        out.media.expect("the sticker image").media_type(),
        pb::MediaType::Sticker
    );
    assert_is_the_clip(&out.sticker_audio.expect("the clip"));
}

#[test]
fn a_plain_sticker_has_no_sticker_audio() {
    let mut msg = wa::Message::default();
    msg.sticker_message = MessageField::some(sticker(false));
    let out = projected(&msg);
    assert!(out.media.is_some(), "the sticker image");
    assert_eq!(out.sticker_audio, None);

    let mut audio = wa::Message::default();
    audio.audio_message = MessageField::some(clip());
    assert_eq!(
        projected(&audio).sticker_audio,
        None,
        "a plain audio is `media`, not a sticker's clip"
    );
}
