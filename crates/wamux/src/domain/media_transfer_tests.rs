//! `media_transfer` (#115): the tests that lived inline, moved so the loop can
//! seal them. Same assertions; the header is the domain's `OutgoingMedia` and
//! the descriptor its `DownloadableMedia`. The `MediaKind` token tests stay as
//! they were (they never took a `pb::*`); reading a descriptor's kind moved
//! to `wamux-types` (`messaging/media_tests.rs`).

use wacore::download::MediaType;
use wamux_types::{
    DownloadableMedia, Jid, MediaKind, MessageId, OutgoingContext, OutgoingMedia, QuotedRef,
};
use whatsapp_rust::waproto::whatsapp as wa;

use super::{MediaUpload, download_params};
use crate::error::WamuxError;

/// The tests build sendable kinds only; the download-only kinds are pinned by
/// `a_download_only_kind_has_no_outgoing_message`.
fn build_media_message(media: &OutgoingMedia, up: MediaUpload) -> wa::Message {
    super::build_media_message(media, up).expect("sendable kind")
}

fn media(kind: MediaKind) -> OutgoingMedia {
    OutgoingMedia {
        kind,
        mime_type: String::new(),
        caption: String::new(),
        filename: String::new(),
        ptt: false,
        seconds: 0,
        waveform: Vec::new(),
        ptv: false,
        context: OutgoingContext::default(),
    }
}

#[test]
fn a_download_only_kind_has_no_outgoing_message() {
    for kind in [MediaKind::StickerPack, MediaKind::StickerPackThumbnail] {
        let err = super::build_media_message(&media(kind), fake_upload());
        assert!(
            matches!(err, Err(WamuxError::InvalidArgument(_))),
            "{kind:?}"
        );
    }
}

#[test]
fn media_kind_parses_each_accepted_string_to_its_upload_type() {
    assert_eq!(
        MediaKind::parse_sendable("image").unwrap().media_type(),
        MediaType::Image
    );
    assert_eq!(
        MediaKind::parse_sendable("video").unwrap().media_type(),
        MediaType::Video
    );
    assert_eq!(
        MediaKind::parse_sendable("audio").unwrap().media_type(),
        MediaType::Audio
    );
    assert_eq!(
        MediaKind::parse_sendable("document").unwrap().media_type(),
        MediaType::Document
    );
    assert_eq!(
        MediaKind::parse_sendable("sticker").unwrap().media_type(),
        MediaType::Sticker
    );
}

#[test]
fn media_kind_unknown_value_is_invalid_argument_with_value() {
    let err = MediaKind::parse_sendable("gif").unwrap_err();
    match err {
        WamuxError::InvalidArgument(msg) => assert!(msg.contains("gif"), "got: {msg}"),
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

// The match arms are exact lowercase literals: "Image" must be rejected.
// Pinned so a future "helpful" case-fold doesn't sneak policy into the core.
#[test]
fn media_kind_is_case_sensitive() {
    assert!(matches!(
        MediaKind::parse_sendable("Image"),
        Err(WamuxError::InvalidArgument(_))
    ));
}

// The pack tokens are download-only: SendMedia parses through MediaKind,
// which must keep refusing them, since there is no sub-message to build.
#[test]
fn send_media_still_refuses_the_pack_tokens() {
    for value in ["sticker_pack", "sticker_pack_thumbnail"] {
        assert!(
            matches!(
                MediaKind::parse_sendable(value),
                Err(WamuxError::InvalidArgument(_))
            ),
            "value: {value}"
        );
    }
}

fn downloadable(media_key: Vec<u8>, file_enc_sha256: Vec<u8>) -> DownloadableMedia {
    DownloadableMedia {
        kind: MediaKind::Image,
        direct_path: "/v/t62.7118-24/enc".to_string(),
        media_key,
        file_enc_sha256,
        file_sha256: vec![9u8; 32],
        file_length: 2048,
        mime_type: "image/jpeg".to_string(),
    }
}

// Ordinary encrypted media: the key rides through and decryption happens.
#[test]
fn a_descriptor_with_a_key_downloads_as_encrypted() {
    let params = download_params(&downloadable(vec![1u8; 32], vec![2u8; 32]));
    assert_eq!(params.media_key.as_deref(), Some(&[1u8; 32][..]));
    assert_eq!(params.file_enc_sha256.as_deref(), Some(&[2u8; 32][..]));
    assert_eq!(params.file_sha256, vec![9u8; 32]);
    assert_eq!(params.media_type, MediaType::Image);
}

// Issue #58: the kind picks the HKDF info, so a pack's thumbnail downloads
// under its own type.
#[test]
fn the_descriptor_kind_picks_the_download_type() {
    let thumbnail = DownloadableMedia {
        kind: MediaKind::StickerPackThumbnail,
        ..downloadable(vec![1u8; 32], vec![2u8; 32])
    };
    assert_eq!(
        download_params(&thumbnail).media_type,
        MediaType::StickerPackThumbnail
    );
}

// REGRESSION (issue #6): a channel's media carries no key at all. It must
// download as plaintext rather than fail, and `file_sha256` must survive —
// it is the only thing authenticating those bytes.
#[test]
fn a_keyless_descriptor_downloads_as_plaintext_and_keeps_its_hash() {
    let params = download_params(&downloadable(Vec::new(), Vec::new()));
    assert!(params.media_key.is_none());
    assert!(params.file_enc_sha256.is_none());
    assert_eq!(params.file_sha256, vec![9u8; 32]);
    assert_eq!(params.file_length, 2048);
}

// An enc hash without a key is not a half-encrypted download: without the
// key there is nothing to decrypt, so carrying it would only invite the
// library to validate a hash of bytes it never produces.
#[test]
fn an_enc_hash_without_a_key_is_dropped() {
    let params = download_params(&downloadable(Vec::new(), vec![2u8; 32]));
    assert!(params.media_key.is_none());
    assert!(params.file_enc_sha256.is_none());
}

fn fake_upload() -> MediaUpload {
    MediaUpload {
        url: "https://mmg.whatsapp.net/v/t62.7118-24/enc".to_string(),
        direct_path: "/v/t62.7118-24/enc".to_string(),
        media_key: [1u8; 32],
        file_enc_sha256: [2u8; 32],
        file_sha256: [3u8; 32],
        file_length: 1234,
        media_key_timestamp: 1_756_800_000,
        // wacore computes one for audio and video only; the fixture carries
        // it so a builder that drops it fails a test (issue #18).
        streaming_sidecar: Some(vec![9u8; 3]),
    }
}

/// An upload of a kind wacore builds no sidecar for.
fn upload_without_sidecar() -> MediaUpload {
    MediaUpload {
        streaming_sidecar: None,
        ..fake_upload()
    }
}

#[test]
fn image_message_carries_upload_mime_and_caption() {
    let message = build_media_message(
        &OutgoingMedia {
            mime_type: "image/jpeg".to_string(),
            caption: "a caption".to_string(),
            ..media(MediaKind::Image)
        },
        fake_upload(),
    );
    let img = message.image_message.expect("image_message must be set");
    assert_eq!(img.url.as_deref(), Some(fake_upload().url.as_str()));
    assert_eq!(img.direct_path.as_deref(), Some("/v/t62.7118-24/enc"));
    assert_eq!(img.media_key.as_deref(), Some(&[1u8; 32][..]));
    assert_eq!(img.file_sha256.as_deref(), Some(&[3u8; 32][..]));
    assert_eq!(img.file_enc_sha256.as_deref(), Some(&[2u8; 32][..]));
    assert_eq!(img.file_length, Some(1234));
    assert_eq!(img.mimetype.as_deref(), Some("image/jpeg"));
    assert_eq!(img.caption.as_deref(), Some("a caption"));
    // No ephemeral, no other context: the ContextInfo stays absent.
    assert!(img.context_info.is_unset());
}

// REGRESSION for issue #18: an AudioMessage without the sidecar draws its
// bubble on the phone and refuses to play, answering `delivered` with no
// nack and nothing in any log. The library asserts the same property for
// its own builder (whatsapp-rust src/media.rs).
#[test]
fn audio_message_carries_the_streaming_sidecar() {
    let message = build_media_message(
        &OutgoingMedia {
            mime_type: "audio/ogg; codecs=opus".to_string(),
            ptt: true,
            seconds: 3,
            ..media(MediaKind::Audio)
        },
        fake_upload(),
    );
    let audio = message.audio_message.expect("audio_message must be set");
    assert_eq!(audio.streaming_sidecar.as_deref(), Some(&[9u8; 3][..]));
    assert_eq!(audio.ptt, Some(true));
}

#[test]
fn video_message_carries_the_streaming_sidecar() {
    let message = build_media_message(&media(MediaKind::Video), fake_upload());
    let video = message.video_message.expect("video_message must be set");
    assert_eq!(video.streaming_sidecar.as_deref(), Some(&[9u8; 3][..]));
}

// A video note rides the same builder, so it must not lose the sidecar on
// the way into the other Message slot.
#[test]
fn a_video_note_keeps_the_streaming_sidecar() {
    let message = build_media_message(
        &OutgoingMedia {
            ptv: true,
            ..media(MediaKind::Video)
        },
        fake_upload(),
    );
    let ptv = message.ptv_message.expect("ptv_message must be set");
    assert_eq!(ptv.streaming_sidecar.as_deref(), Some(&[9u8; 3][..]));
    assert!(message.video_message.is_unset());
}

// Absence must survive too: wacore returns no sidecar for the kinds fetched
// whole, and the field is then absent rather than Some(empty).
#[test]
fn no_sidecar_from_the_upload_means_an_absent_field() {
    let message = build_media_message(&media(MediaKind::Audio), upload_without_sidecar());
    let audio = message.audio_message.expect("audio_message must be set");
    assert_eq!(audio.streaming_sidecar, None);
}

// Every media sub-message carries a media_key_timestamp; the shared macro
// sets it, so one test over two kinds pins the whole table.
#[test]
fn media_key_timestamp_reaches_every_kind() {
    let image = build_media_message(&media(MediaKind::Image), fake_upload())
        .image_message
        .expect("image_message must be set");
    assert_eq!(image.media_key_timestamp, Some(1_756_800_000));
    let document = build_media_message(&media(MediaKind::Document), fake_upload())
        .document_message
        .expect("document_message must be set");
    assert_eq!(document.media_key_timestamp, Some(1_756_800_000));
}

#[test]
fn document_message_carries_filename_and_caption() {
    let message = build_media_message(
        &OutgoingMedia {
            mime_type: "application/pdf".to_string(),
            caption: "the report".to_string(),
            filename: "report.pdf".to_string(),
            ..media(MediaKind::Document)
        },
        fake_upload(),
    );
    let doc = message
        .document_message
        .expect("document_message must be set");
    assert_eq!(doc.mimetype.as_deref(), Some("application/pdf"));
    assert_eq!(doc.file_name.as_deref(), Some("report.pdf"));
    assert_eq!(doc.caption.as_deref(), Some("the report"));
    assert_eq!(doc.file_length, Some(1234));
    // The other sub-messages must stay unset: exactly one media branch.
    assert!(message.image_message.is_unset());
    assert!(message.video_message.is_unset());
}

#[test]
fn audio_with_ptt_seconds_waveform_is_a_voice_note() {
    let message = build_media_message(
        &OutgoingMedia {
            mime_type: "audio/ogg; codecs=opus".to_string(),
            ptt: true,
            seconds: 17,
            waveform: vec![0u8, 50, 100],
            ..media(MediaKind::Audio)
        },
        fake_upload(),
    );
    let audio = message.audio_message.expect("audio_message must be set");
    assert_eq!(audio.ptt, Some(true));
    assert_eq!(audio.seconds, Some(17));
    assert_eq!(audio.waveform.as_deref(), Some(&[0u8, 50, 100][..]));
    assert_eq!(audio.mimetype.as_deref(), Some("audio/ogg; codecs=opus"));
}

// Pinned: "not a voice note" is the ABSENT field, never Some(false), and
// zero seconds / empty waveform (proto3 defaults) stay absent too.
#[test]
fn audio_without_ptt_stays_plain_audio() {
    let message = build_media_message(
        &OutgoingMedia {
            mime_type: "audio/mp4".to_string(),
            ..media(MediaKind::Audio)
        },
        fake_upload(),
    );
    let audio = message.audio_message.expect("audio_message must be set");
    assert_eq!(audio.ptt, None);
    assert_eq!(audio.seconds, None);
    assert_eq!(audio.waveform, None);
    assert!(audio.context_info.is_unset());
}

#[test]
fn audio_ptt_with_empty_waveform_maps_waveform_to_none() {
    let message = build_media_message(
        &OutgoingMedia {
            ptt: true,
            seconds: 3,
            waveform: vec![],
            ..media(MediaKind::Audio)
        },
        fake_upload(),
    );
    let audio = message.audio_message.expect("audio_message must be set");
    assert_eq!(audio.ptt, Some(true));
    assert_eq!(audio.waveform, None);
}

// PTV (video note): the built VideoMessage must land in ptv_message, not
// video_message — same submessage, different Message slot.
#[test]
fn ptv_video_routes_into_ptv_message() {
    let message = build_media_message(
        &OutgoingMedia {
            mime_type: "video/mp4".to_string(),
            ptv: true,
            ..media(MediaKind::Video)
        },
        fake_upload(),
    );
    let ptv = message.ptv_message.expect("ptv_message must be set");
    assert_eq!(ptv.url.as_deref(), Some(fake_upload().url.as_str()));
    assert_eq!(ptv.mimetype.as_deref(), Some("video/mp4"));
    // The regular video slot must stay empty: exactly one branch fires.
    assert!(message.video_message.is_unset());
}

// Without the ptv flag a video stays a normal video_message.
#[test]
fn non_ptv_video_stays_video_message() {
    let message = build_media_message(
        &OutgoingMedia {
            mime_type: "video/mp4".to_string(),
            ..media(MediaKind::Video)
        },
        fake_upload(),
    );
    assert!(message.video_message.is_set());
    assert!(message.ptv_message.is_unset());
}

#[test]
fn ephemeral_image_sets_context_expiration() {
    let message = build_media_message(
        &OutgoingMedia {
            context: OutgoingContext {
                ephemeral_seconds: 86_400,
                ..Default::default()
            },
            ..media(MediaKind::Image)
        },
        fake_upload(),
    );
    let img = message.image_message.expect("image_message must be set");
    let context = img.context_info.expect("context_info must be set");
    assert_eq!(context.expiration, Some(86_400));
}

// Regression (code-review 2026-06-11): SendMediaHeader.quote and .mentions
// were silently dropped — only ephemeral reached the ContextInfo. A media
// reply must carry the quote exactly like the text path does.
#[test]
fn media_quote_and_mentions_relay_into_context() {
    let message = build_media_message(
        &OutgoingMedia {
            context: OutgoingContext {
                mentions: vec![Jid::parse("5511888888888@s.whatsapp.net").unwrap()],
                quote: Some(QuotedRef {
                    id: MessageId::new("QUOTED-1").unwrap(),
                    participant: Some(Jid::parse("5511777777777@s.whatsapp.net").unwrap()),
                }),
                ephemeral_seconds: 90,
            },
            ..media(MediaKind::Image)
        },
        fake_upload(),
    );
    let img = message.image_message.expect("image_message must be set");
    let context = img.context_info.expect("context_info must be set");
    assert_eq!(
        context.mentioned_jid,
        vec!["5511888888888@s.whatsapp.net".to_string()]
    );
    assert_eq!(context.stanza_id.as_deref(), Some("QUOTED-1"));
    assert_eq!(
        context.participant.as_deref(),
        Some("5511777777777@s.whatsapp.net")
    );
    // Quote/mentions compose with ephemeral in the one shared ContextInfo.
    assert_eq!(context.expiration, Some(90));
}

// Every media branch must relay the expiration, not just image/audio.
#[test]
fn ephemeral_applies_to_every_media_branch() {
    let kinds = ["video", "audio", "document", "sticker"];
    for kind in kinds {
        let message = build_media_message(
            &OutgoingMedia {
                context: OutgoingContext {
                    ephemeral_seconds: 604_800,
                    ..Default::default()
                },
                ..media(MediaKind::parse_sendable(kind).expect("test kinds are valid"))
            },
            fake_upload(),
        );
        let expiration = match kind {
            "video" => message.video_message.unwrap().context_info,
            "audio" => message.audio_message.unwrap().context_info,
            "document" => message.document_message.unwrap().context_info,
            _ => message.sticker_message.unwrap().context_info,
        }
        .expect("context_info must be set")
        .expiration;
        assert_eq!(expiration, Some(604_800), "branch: {kind}");
    }
}
