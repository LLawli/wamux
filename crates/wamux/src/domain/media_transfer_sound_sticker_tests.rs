//! `build_sound_sticker_message` (#278): the sticker and its clip, each with
//! its own upload, inside the `audioStickerMessage` wrapper.

use wacore::proto_helpers::MessageExt as _;
use wamux_types::{Jid, MediaKind, OutgoingContext, OutgoingMedia, StickerAudio};
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::waproto::whatsapp::message::sticker_message::Audio;

use super::{MediaUpload, build_sound_sticker_message};

const MENTIONED: &str = "5511888888888@s.whatsapp.net";

fn sound_sticker() -> OutgoingMedia {
    OutgoingMedia {
        kind: MediaKind::Sticker,
        mime_type: "image/webp".to_string(),
        caption: String::new(),
        filename: String::new(),
        ptt: false,
        seconds: 0,
        waveform: Vec::new(),
        ptv: false,
        context: OutgoingContext {
            mentions: vec![Jid::parse(MENTIONED).unwrap()],
            quote: None,
            ephemeral_seconds: 0,
        },
        sticker_audio: Some(StickerAudio {
            mime_type: "audio/ogg; codecs=opus".to_string(),
            seconds: 3,
        }),
    }
}

/// The image's upload: wacore builds no sidecar for a sticker.
fn sticker_upload() -> MediaUpload {
    MediaUpload {
        url: "https://mmg.whatsapp.net/v/t62.15575-24/sticker".to_string(),
        direct_path: "/v/t62.15575-24/sticker".to_string(),
        media_key: [1u8; 32],
        file_enc_sha256: [2u8; 32],
        file_sha256: [3u8; 32],
        file_length: 20_000,
        media_key_timestamp: 1_795_000_000,
        streaming_sidecar: None,
    }
}

/// The clip's upload, with the sidecar wacore computes for audio (#18).
fn clip_upload() -> MediaUpload {
    MediaUpload {
        url: "https://mmg.whatsapp.net/v/t62.7114-24/clip".to_string(),
        direct_path: "/v/t62.7114-24/clip".to_string(),
        media_key: [4u8; 32],
        file_enc_sha256: [5u8; 32],
        file_sha256: [6u8; 32],
        file_length: 9_000,
        media_key_timestamp: 1_795_000_001,
        streaming_sidecar: Some(vec![7u8; 10]),
    }
}

fn built() -> wa::Message {
    build_sound_sticker_message(&sound_sticker(), sticker_upload(), clip_upload())
        .expect("a sound sticker builds")
}

#[test]
fn a_sound_sticker_wraps_the_sticker_with_its_clip() {
    let message = built();
    assert!(
        message.sticker_message.is_unset(),
        "the sticker rides in the wrapper, not at the top"
    );
    let wrapper = message
        .audio_sticker_message
        .as_option()
        .expect("audioStickerMessage");
    let inner = wrapper.message.as_option().expect("the wrapped message");
    let sticker = inner.sticker_message.as_option().expect("stickerMessage");

    assert_eq!(
        sticker.direct_path.as_deref(),
        Some("/v/t62.15575-24/sticker")
    );
    assert_eq!(
        sticker.url.as_deref(),
        Some("https://mmg.whatsapp.net/v/t62.15575-24/sticker")
    );
    assert_eq!(sticker.media_key.as_deref(), Some(&[1u8; 32][..]));
    assert_eq!(sticker.file_enc_sha256.as_deref(), Some(&[2u8; 32][..]));
    assert_eq!(sticker.file_sha256.as_deref(), Some(&[3u8; 32][..]));
    assert_eq!(sticker.file_length, Some(20_000));
    assert_eq!(sticker.media_key_timestamp, Some(1_795_000_000));
    assert_eq!(sticker.mimetype.as_deref(), Some("image/webp"));
    let context = sticker
        .context_info
        .as_option()
        .expect("the sticker's context");
    assert_eq!(context.mentioned_jid, vec![MENTIONED.to_string()]);

    let Some(Audio::AudioMessage(clip)) = sticker.audio.as_ref() else {
        panic!("the sticker carries its clip: {:?}", sticker.audio);
    };
    assert_eq!(clip.direct_path.as_deref(), Some("/v/t62.7114-24/clip"));
    assert_eq!(
        clip.url.as_deref(),
        Some("https://mmg.whatsapp.net/v/t62.7114-24/clip")
    );
    assert_eq!(clip.media_key.as_deref(), Some(&[4u8; 32][..]));
    assert_eq!(clip.file_enc_sha256.as_deref(), Some(&[5u8; 32][..]));
    assert_eq!(clip.file_sha256.as_deref(), Some(&[6u8; 32][..]));
    assert_eq!(clip.file_length, Some(9_000));
    assert_eq!(clip.media_key_timestamp, Some(1_795_000_001));
    assert_eq!(clip.mimetype.as_deref(), Some("audio/ogg; codecs=opus"));
    assert_eq!(clip.seconds, Some(3));
    assert_eq!(
        clip.streaming_sidecar.as_deref(),
        Some(&[7u8; 10][..]),
        "without the sidecar the clip does not play (#18)"
    );
    assert_eq!(clip.ptt, None, "a sound sticker's clip is not a voice note");
    assert!(
        clip.context_info.is_unset(),
        "the context belongs to the sticker"
    );
}

/// The library classifies a send by its base message: the wrapper must peel
/// down to the sticker, or the stanza leaves with no `mediatype`.
#[test]
fn the_sound_sticker_base_is_the_sticker() {
    let message = built();
    let base = message.get_base_message();
    assert!(base.sticker_message.is_set(), "{base:?}");
}
