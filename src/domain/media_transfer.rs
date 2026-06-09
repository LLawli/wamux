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
