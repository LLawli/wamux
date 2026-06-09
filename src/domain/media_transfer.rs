//! Upload media for sending and download received media (lazy, from descriptor).

use wacore::download::MediaType;
use whatsapp_rust::upload::{UploadOptions, UploadResponse};
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::{Client, Jid, SendResult};

use crate::error::WamuxError;
use crate::proto::v1 as pb;

fn client_err<E: std::fmt::Display>(e: E) -> WamuxError {
    WamuxError::Client(format!("{e:#}"))
}

fn media_type(value: &str) -> Result<MediaType, WamuxError> {
    Ok(match value {
        "image" => MediaType::Image,
        "video" => MediaType::Video,
        "audio" => MediaType::Audio,
        "document" => MediaType::Document,
        "sticker" => MediaType::Sticker,
        other => {
            return Err(WamuxError::InvalidArgument(format!(
                "unknown media_type '{other}'"
            )));
        }
    })
}

pub async fn send_media(
    client: &Client,
    to: Jid,
    header: &pb::SendMediaHeader,
    data: Vec<u8>,
) -> Result<SendResult, WamuxError> {
    let kind = media_type(&header.media_type)?;
    let upload = client
        .upload(data, kind, UploadOptions::new())
        .await
        .map_err(client_err)?;
    let mime = (!header.mime_type.is_empty()).then(|| header.mime_type.clone());
    let caption = (!header.caption.is_empty()).then(|| header.caption.clone());
    let filename = (!header.filename.is_empty()).then(|| header.filename.clone());
    let message = build_media_message(&header.media_type, upload, mime, caption, filename);
    client.send_message(to, message).await.map_err(client_err)
}

fn build_media_message(
    media_type: &str,
    up: UploadResponse,
    mime: Option<String>,
    caption: Option<String>,
    filename: Option<String>,
) -> wa::Message {
    match media_type {
        "video" => wa::Message {
            video_message: Some(Box::new(wa::message::VideoMessage {
                url: Some(up.url),
                direct_path: Some(up.direct_path),
                media_key: Some(up.media_key.to_vec()),
                file_sha256: Some(up.file_sha256.to_vec()),
                file_enc_sha256: Some(up.file_enc_sha256.to_vec()),
                file_length: Some(up.file_length),
                mimetype: mime,
                caption,
                ..Default::default()
            })),
            ..Default::default()
        },
        "audio" => wa::Message {
            audio_message: Some(Box::new(wa::message::AudioMessage {
                url: Some(up.url),
                direct_path: Some(up.direct_path),
                media_key: Some(up.media_key.to_vec()),
                file_sha256: Some(up.file_sha256.to_vec()),
                file_enc_sha256: Some(up.file_enc_sha256.to_vec()),
                file_length: Some(up.file_length),
                mimetype: mime,
                ..Default::default()
            })),
            ..Default::default()
        },
        "document" => wa::Message {
            document_message: Some(Box::new(wa::message::DocumentMessage {
                url: Some(up.url),
                direct_path: Some(up.direct_path),
                media_key: Some(up.media_key.to_vec()),
                file_sha256: Some(up.file_sha256.to_vec()),
                file_enc_sha256: Some(up.file_enc_sha256.to_vec()),
                file_length: Some(up.file_length),
                mimetype: mime,
                file_name: filename,
                caption,
                ..Default::default()
            })),
            ..Default::default()
        },
        "sticker" => wa::Message {
            sticker_message: Some(Box::new(wa::message::StickerMessage {
                url: Some(up.url),
                direct_path: Some(up.direct_path),
                media_key: Some(up.media_key.to_vec()),
                file_sha256: Some(up.file_sha256.to_vec()),
                file_enc_sha256: Some(up.file_enc_sha256.to_vec()),
                file_length: Some(up.file_length),
                mimetype: mime,
                ..Default::default()
            })),
            ..Default::default()
        },
        _ => wa::Message {
            image_message: Some(Box::new(wa::message::ImageMessage {
                url: Some(up.url),
                direct_path: Some(up.direct_path),
                media_key: Some(up.media_key.to_vec()),
                file_sha256: Some(up.file_sha256.to_vec()),
                file_enc_sha256: Some(up.file_enc_sha256.to_vec()),
                file_length: Some(up.file_length),
                mimetype: mime,
                caption,
                ..Default::default()
            })),
            ..Default::default()
        },
    }
}

/// Lazy download from a descriptor the edge got off an inbound message event.
pub async fn download(
    client: &Client,
    descriptor: &pb::MediaDescriptor,
) -> Result<Vec<u8>, WamuxError> {
    let kind = media_type(&descriptor.media_type)?;
    client
        .download_from_params(
            &descriptor.direct_path,
            &descriptor.media_key,
            &descriptor.file_sha256,
            &descriptor.file_enc_sha256,
            descriptor.file_length,
            kind,
        )
        .await
        .map_err(client_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_type_maps_each_accepted_string() {
        assert_eq!(media_type("image").unwrap(), MediaType::Image);
        assert_eq!(media_type("video").unwrap(), MediaType::Video);
        assert_eq!(media_type("audio").unwrap(), MediaType::Audio);
        assert_eq!(media_type("document").unwrap(), MediaType::Document);
        assert_eq!(media_type("sticker").unwrap(), MediaType::Sticker);
    }

    #[test]
    fn media_type_unknown_value_is_invalid_argument_with_value() {
        let err = media_type("gif").unwrap_err();
        match err {
            WamuxError::InvalidArgument(msg) => assert!(msg.contains("gif"), "got: {msg}"),
            other => panic!("expected InvalidArgument, got {other:?}"),
        }
    }

    // The match arms are exact lowercase literals: "Image" must be rejected.
    // Pinned so a future "helpful" case-fold doesn't sneak policy into the core.
    #[test]
    fn media_type_is_case_sensitive() {
        assert!(matches!(
            media_type("Image"),
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

    #[test]
    fn image_message_carries_upload_mime_and_caption() {
        let message = build_media_message(
            "image",
            fake_upload(),
            Some("image/jpeg".to_string()),
            Some("a caption".to_string()),
            None,
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
    }

    #[test]
    fn document_message_carries_filename_and_caption() {
        let message = build_media_message(
            "document",
            fake_upload(),
            Some("application/pdf".to_string()),
            Some("the report".to_string()),
            Some("report.pdf".to_string()),
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
}
