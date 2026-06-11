//! Upload media for sending and download received media (lazy, from descriptor).

use wacore::download::MediaType;
use whatsapp_rust::upload::{UploadOptions, UploadResponse};
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::waproto::whatsapp::message::{
    AudioMessage, DocumentMessage, ImageMessage, StickerMessage, VideoMessage,
};
use whatsapp_rust::{Client, Jid, SendResult};

use crate::domain::outgoing_context::outgoing_context;
use crate::domain::wire_defaults::{nonempty_bytes, nonempty_string, nonzero_u32};
use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

/// The five media kinds wamux relays, parsed ONCE from the wire string. Both
/// the upload `MediaType` and the outgoing sub-message derive from this enum,
/// so the two can never drift (code-review 2026-06-11: two independent string
/// matches let a payload upload under one type yet ship mislabeled as image).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MediaKind {
    Image,
    Video,
    Audio,
    Document,
    Sticker,
}

impl MediaKind {
    pub(crate) fn parse(value: &str) -> Result<Self, WamuxError> {
        Ok(match value {
            "image" => Self::Image,
            "video" => Self::Video,
            "audio" => Self::Audio,
            "document" => Self::Document,
            "sticker" => Self::Sticker,
            other => {
                return Err(WamuxError::InvalidArgument(format!(
                    "unknown media_type '{other}'"
                )));
            }
        })
    }

    /// The wacore upload/download type for this kind (wacore's enum has many
    /// more variants — history, app state — that are not relay media).
    fn upload_type(self) -> MediaType {
        match self {
            Self::Image => MediaType::Image,
            Self::Video => MediaType::Video,
            Self::Audio => MediaType::Audio,
            Self::Document => MediaType::Document,
            Self::Sticker => MediaType::Sticker,
        }
    }
}

pub async fn send_media(
    client: &Client,
    to: Jid,
    header: &pb::SendMediaHeader,
    data: Vec<u8>,
) -> Result<SendResult, WamuxError> {
    let kind = MediaKind::parse(&header.media_type)?;
    let upload = client
        .upload(data, kind.upload_type(), UploadOptions::new())
        .await
        .map_err(client_err)?;
    let message = build_media_message(kind, header, upload);
    client.send_message(to, message).await.map_err(client_err)
}

/// Pure construction of the outgoing media `wa::Message` from the wire header
/// plus the finished upload. Header fields relay verbatim; proto3 defaults
/// (empty string/bytes, zero) map to absent waproto fields. The exhaustive
/// `MediaKind` match (no catch-all) keeps builder and upload type in lockstep.
pub(crate) fn build_media_message(
    kind: MediaKind,
    header: &pb::SendMediaHeader,
    up: UploadResponse,
) -> wa::Message {
    // Each wa media sub-message carries its own ContextInfo; the shared
    // builder relays mentions + quote + ephemeral (or omits the field when
    // all three are the proto3 default), same composition as the text path.
    let context = outgoing_context(
        &header.mentions,
        header.quote.as_ref(),
        header.ephemeral_seconds,
    );
    match kind {
        MediaKind::Image => wa::Message {
            image_message: Some(Box::new(image_submessage(header, up, context))),
            ..Default::default()
        },
        MediaKind::Video => wa::Message {
            video_message: Some(Box::new(video_submessage(header, up, context))),
            ..Default::default()
        },
        MediaKind::Audio => wa::Message {
            audio_message: Some(Box::new(audio_submessage(header, up, context))),
            ..Default::default()
        },
        MediaKind::Document => wa::Message {
            document_message: Some(Box::new(document_submessage(header, up, context))),
            ..Default::default()
        },
        MediaKind::Sticker => wa::Message {
            sticker_message: Some(Box::new(sticker_submessage(header, up, context))),
            ..Default::default()
        },
    }
}

/// The five wa media sub-messages duplicate the exact same upload/mime/context
/// field names, but prost generates no shared trait to abstract over them —
/// this macro expands the struct literal so each builder states only its
/// type-specific fields.
macro_rules! submessage_with_upload {
    ($ty:ident { $($field:ident : $value:expr),* $(,)? }, $header:expr, $up:expr, $context:expr) => {
        $ty {
            url: Some($up.url),
            direct_path: Some($up.direct_path),
            media_key: Some($up.media_key.to_vec()),
            file_sha256: Some($up.file_sha256.to_vec()),
            file_enc_sha256: Some($up.file_enc_sha256.to_vec()),
            file_length: Some($up.file_length),
            mimetype: nonempty_string(&$header.mime_type),
            context_info: $context,
            $($field: $value,)*
            ..Default::default()
        }
    };
}

fn video_submessage(
    header: &pb::SendMediaHeader,
    up: UploadResponse,
    context: Option<Box<wa::ContextInfo>>,
) -> wa::message::VideoMessage {
    submessage_with_upload!(
        VideoMessage {
            caption: nonempty_string(&header.caption),
        },
        header,
        up,
        context
    )
}

fn audio_submessage(
    header: &pb::SendMediaHeader,
    up: UploadResponse,
    context: Option<Box<wa::ContextInfo>>,
) -> wa::message::AudioMessage {
    submessage_with_upload!(
        AudioMessage {
            // Voice note: relayed flags only. WhatsApp renders PTT solely for
            // OGG/Opus payloads; supplying those bytes is the edge's job.
            // "Not a voice note" is the absent field (None), never Some(false).
            ptt: header.ptt.then_some(true),
            seconds: nonzero_u32(header.seconds),
            waveform: nonempty_bytes(&header.waveform),
        },
        header,
        up,
        context
    )
}

fn document_submessage(
    header: &pb::SendMediaHeader,
    up: UploadResponse,
    context: Option<Box<wa::ContextInfo>>,
) -> wa::message::DocumentMessage {
    submessage_with_upload!(
        DocumentMessage {
            file_name: nonempty_string(&header.filename),
            caption: nonempty_string(&header.caption),
        },
        header,
        up,
        context
    )
}

fn sticker_submessage(
    header: &pb::SendMediaHeader,
    up: UploadResponse,
    context: Option<Box<wa::ContextInfo>>,
) -> wa::message::StickerMessage {
    submessage_with_upload!(StickerMessage {}, header, up, context)
}

fn image_submessage(
    header: &pb::SendMediaHeader,
    up: UploadResponse,
    context: Option<Box<wa::ContextInfo>>,
) -> wa::message::ImageMessage {
    submessage_with_upload!(
        ImageMessage {
            caption: nonempty_string(&header.caption),
        },
        header,
        up,
        context
    )
}

/// Lazy download from a descriptor the edge got off an inbound message event.
pub async fn download(
    client: &Client,
    descriptor: &pb::MediaDescriptor,
) -> Result<Vec<u8>, WamuxError> {
    let kind = MediaKind::parse(&descriptor.media_type)?;
    client
        .download_from_params(
            &descriptor.direct_path,
            &descriptor.media_key,
            &descriptor.file_sha256,
            &descriptor.file_enc_sha256,
            descriptor.file_length,
            kind.upload_type(),
        )
        .await
        .map_err(client_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_kind_parses_each_accepted_string_to_its_upload_type() {
        assert_eq!(
            MediaKind::parse("image").unwrap().upload_type(),
            MediaType::Image
        );
        assert_eq!(
            MediaKind::parse("video").unwrap().upload_type(),
            MediaType::Video
        );
        assert_eq!(
            MediaKind::parse("audio").unwrap().upload_type(),
            MediaType::Audio
        );
        assert_eq!(
            MediaKind::parse("document").unwrap().upload_type(),
            MediaType::Document
        );
        assert_eq!(
            MediaKind::parse("sticker").unwrap().upload_type(),
            MediaType::Sticker
        );
    }

    #[test]
    fn media_kind_unknown_value_is_invalid_argument_with_value() {
        let err = MediaKind::parse("gif").unwrap_err();
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
            MediaKind::parse("Image"),
            Err(WamuxError::InvalidArgument(_))
        ));
    }

    fn fake_upload() -> UploadResponse {
        UploadResponse {
            url: "https://mmg.whatsapp.net/v/t62.7118-24/enc".to_string(),
            direct_path: "/v/t62.7118-24/enc".to_string(),
            media_key: [1u8; 32],
            file_enc_sha256: [2u8; 32],
            file_sha256: [3u8; 32],
            file_length: 1234,
            media_key_timestamp: 1_700_000_000,
        }
    }

    fn header(media_type: &str) -> pb::SendMediaHeader {
        pb::SendMediaHeader {
            media_type: media_type.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn image_message_carries_upload_mime_and_caption() {
        let message = build_media_message(
            MediaKind::Image,
            &pb::SendMediaHeader {
                mime_type: "image/jpeg".to_string(),
                caption: "a caption".to_string(),
                ..header("image")
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
        assert!(img.context_info.is_none());
    }

    #[test]
    fn document_message_carries_filename_and_caption() {
        let message = build_media_message(
            MediaKind::Document,
            &pb::SendMediaHeader {
                mime_type: "application/pdf".to_string(),
                caption: "the report".to_string(),
                filename: "report.pdf".to_string(),
                ..header("document")
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
        assert!(message.image_message.is_none());
        assert!(message.video_message.is_none());
    }

    #[test]
    fn audio_with_ptt_seconds_waveform_is_a_voice_note() {
        let message = build_media_message(
            MediaKind::Audio,
            &pb::SendMediaHeader {
                mime_type: "audio/ogg; codecs=opus".to_string(),
                ptt: true,
                seconds: 17,
                waveform: vec![0u8, 50, 100],
                ..header("audio")
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
            MediaKind::Audio,
            &pb::SendMediaHeader {
                mime_type: "audio/mp4".to_string(),
                ..header("audio")
            },
            fake_upload(),
        );
        let audio = message.audio_message.expect("audio_message must be set");
        assert_eq!(audio.ptt, None);
        assert_eq!(audio.seconds, None);
        assert_eq!(audio.waveform, None);
        assert!(audio.context_info.is_none());
    }

    #[test]
    fn audio_ptt_with_empty_waveform_maps_waveform_to_none() {
        let message = build_media_message(
            MediaKind::Audio,
            &pb::SendMediaHeader {
                ptt: true,
                seconds: 3,
                waveform: vec![],
                ..header("audio")
            },
            fake_upload(),
        );
        let audio = message.audio_message.expect("audio_message must be set");
        assert_eq!(audio.ptt, Some(true));
        assert_eq!(audio.waveform, None);
    }

    #[test]
    fn ephemeral_image_sets_context_expiration() {
        let message = build_media_message(
            MediaKind::Image,
            &pb::SendMediaHeader {
                ephemeral_seconds: 86_400,
                ..header("image")
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
            MediaKind::Image,
            &pb::SendMediaHeader {
                mentions: vec![pb::Mention {
                    jid: "5511888888888@s.whatsapp.net".to_string(),
                }],
                quote: Some(pb::QuoteContext {
                    quoted: Some(pb::MessageKey {
                        remote_jid: "120363001234567890@g.us".to_string(),
                        id: "QUOTED-1".to_string(),
                        from_me: false,
                        participant: "5511777777777@s.whatsapp.net".to_string(),
                    }),
                    participant: String::new(),
                }),
                ephemeral_seconds: 90,
                ..header("image")
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
                MediaKind::parse(kind).expect("test kinds are valid"),
                &pb::SendMediaHeader {
                    ephemeral_seconds: 604_800,
                    ..header(kind)
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
}
