//! Upload media for sending and download received media (lazy, from descriptor).

use wamux_types::{DownloadableMedia, Jid, MediaKind, OutgoingMedia};
use whatsapp_rust::buffa;
use whatsapp_rust::download::DownloadParams;
use whatsapp_rust::upload::{UploadOptions, UploadResponse};
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::waproto::whatsapp::message::{
    AudioMessage, DocumentMessage, ImageMessage, StickerMessage, VideoMessage,
};
use whatsapp_rust::{Client, SendResult};

use crate::domain::outgoing_context::outgoing_context;
use crate::domain::wire_defaults::{nonempty_bytes, nonempty_string, nonzero_u32};
use crate::error::{WamuxError, client_err};

pub async fn send_media(
    client: &Client,
    to: Jid,
    media: &OutgoingMedia,
    data: Vec<u8>,
) -> Result<SendResult, WamuxError> {
    let upload = client
        .upload(data, media.kind.media_type(), UploadOptions::new())
        .await
        .map_err(client_err)?;
    let message = build_media_message(media, upload.into())?;
    client
        .send_message(to.into_lib(), message)
        .await
        .map_err(client_err)
}

/// The upload fields the outgoing sub-messages need, owned by wamux.
///
/// `UploadResponse` became `#[non_exhaustive]` in whatsapp-rust 0.7 with no
/// public constructor, so it can no longer be built outside the crate: the
/// builders below would be untestable if they took it directly. Naming the six
/// fields the core actually relays also keeps a new upstream field from
/// silently widening what the core copies onto the wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MediaUpload {
    pub url: String,
    pub direct_path: String,
    pub media_key: [u8; 32],
    pub file_sha256: [u8; 32],
    pub file_enc_sha256: [u8; 32],
    pub file_length: u64,
    /// Unix seconds the media key was generated. Every media sub-message
    /// carries one, so the shared macro sets it (issue #18).
    pub media_key_timestamp: i64,
    /// Per-64-KiB HMAC table the official client uses to decrypt progressively
    /// while playing. `wacore` computes it for AUDIO and VIDEO only, so it is
    /// `None` for the kinds that are fetched whole -- and it is set in the
    /// audio/video builders, never in the shared macro, because
    /// `streamingSidecar` exists on only those two waproto messages.
    ///
    /// Issue #18: dropping this shipped audio and voice notes that drew their
    /// bubble and refused to play, with a `delivered` receipt and no nack. The
    /// bytes, the container, the encryption and the descriptor were all right;
    /// the message was missing the table.
    pub streaming_sidecar: Option<Vec<u8>>,
}

impl From<UploadResponse> for MediaUpload {
    fn from(up: UploadResponse) -> Self {
        Self {
            url: up.url,
            direct_path: up.direct_path,
            media_key: up.media_key,
            file_sha256: up.file_sha256,
            file_enc_sha256: up.file_enc_sha256,
            file_length: up.file_length,
            media_key_timestamp: up.media_key_timestamp,
            streaming_sidecar: up.streaming_sidecar,
        }
    }
}

/// Pure construction of the outgoing media `wa::Message` from the parsed header
/// plus the finished upload. Header fields relay verbatim; proto3 defaults
/// (empty string/bytes, zero) map to absent waproto fields. The exhaustive
/// `MediaKind` match (no catch-all) keeps builder and upload type in lockstep.
///
/// `MediaKind` also names the two download-only sticker pack kinds (#114);
/// they have no outgoing sub-message, so they are refused here the way
/// `parse_sendable` refuses them, rather than shipping an empty message.
pub(crate) fn build_media_message(
    header: &OutgoingMedia,
    up: MediaUpload,
) -> Result<wa::Message, WamuxError> {
    // Each wa media sub-message carries its own ContextInfo; the shared
    // builder relays mentions + quote + ephemeral (or omits the field when
    // all three are the proto3 default), same composition as the text path.
    let context = outgoing_context(&header.context);
    let kind = header.kind;
    let message = match kind {
        MediaKind::Image => wa::Message {
            image_message: buffa::MessageField::some(image_submessage(header, up, context)),
            ..Default::default()
        },
        // PTV (video note, the round "instant video") is the SAME VideoMessage
        // wire shape, just carried in a different Message slot. The edge sets
        // header.ptv; the core only relays it into ptv_message vs video_message.
        MediaKind::Video if header.ptv => wa::Message {
            ptv_message: buffa::MessageField::some(video_submessage(header, up, context)),
            ..Default::default()
        },
        MediaKind::Video => wa::Message {
            video_message: buffa::MessageField::some(video_submessage(header, up, context)),
            ..Default::default()
        },
        MediaKind::Audio => wa::Message {
            audio_message: buffa::MessageField::some(audio_submessage(header, up, context)),
            ..Default::default()
        },
        MediaKind::Document => wa::Message {
            document_message: buffa::MessageField::some(document_submessage(header, up, context)),
            ..Default::default()
        },
        MediaKind::Sticker => wa::Message {
            sticker_message: buffa::MessageField::some(sticker_submessage(header, up, context)),
            ..Default::default()
        },
        MediaKind::StickerPack | MediaKind::StickerPackThumbnail => {
            return Err(WamuxError::InvalidArgument(format!(
                "unknown media_type '{}'",
                kind.token()
            )));
        }
    };
    Ok(message)
}

/// The five wa media sub-messages duplicate the exact same upload/mime/context
/// field names, but the protobuf generator (buffa since 0.7) emits no shared
/// trait to abstract over them, so this macro expands the struct literal and
/// each builder states only its type-specific fields.
macro_rules! submessage_with_upload {
    ($ty:ident { $($field:ident : $value:expr),* $(,)? }, $header:expr, $up:expr, $context:expr) => {
        $ty {
            url: Some($up.url),
            direct_path: Some($up.direct_path),
            media_key: Some($up.media_key.to_vec()),
            file_sha256: Some($up.file_sha256.to_vec()),
            file_enc_sha256: Some($up.file_enc_sha256.to_vec()),
            file_length: Some($up.file_length),
            // Unconditional `Some`, matching the library's own builders: this
            // comes from the finished upload, not from the wire, so it is never
            // the proto3 default the wire-defaults rule is about.
            media_key_timestamp: Some($up.media_key_timestamp),
            mimetype: nonempty_string(&$header.mime_type),
            context_info: $context,
            $($field: $value,)*
            ..Default::default()
        }
    };
}

fn video_submessage(
    header: &OutgoingMedia,
    up: MediaUpload,
    context: buffa::MessageField<wa::ContextInfo>,
) -> wa::message::VideoMessage {
    submessage_with_upload!(
        VideoMessage {
            caption: nonempty_string(&header.caption),
            // Issue #18, same reason as the voice note: audio and video are the
            // only two waproto messages with this field, and the only two
            // wacore computes a sidecar for. PTV rides this builder too, so a
            // video note gets it as well.
            streaming_sidecar: up.streaming_sidecar.clone(),
        },
        header,
        up,
        context
    )
}

fn audio_submessage(
    header: &OutgoingMedia,
    up: MediaUpload,
    context: buffa::MessageField<wa::ContextInfo>,
) -> wa::message::AudioMessage {
    submessage_with_upload!(
        AudioMessage {
            // Voice note: relayed flags only. WhatsApp renders PTT solely for
            // OGG/Opus payloads; supplying those bytes is the edge's job.
            // "Not a voice note" is the absent field (None), never Some(false).
            ptt: header.ptt.then_some(true),
            seconds: nonzero_u32(header.seconds),
            waveform: nonempty_bytes(&header.waveform),
            // Issue #18: without this the bubble draws and will not play.
            streaming_sidecar: up.streaming_sidecar.clone(),
        },
        header,
        up,
        context
    )
}

fn document_submessage(
    header: &OutgoingMedia,
    up: MediaUpload,
    context: buffa::MessageField<wa::ContextInfo>,
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
    header: &OutgoingMedia,
    up: MediaUpload,
    context: buffa::MessageField<wa::ContextInfo>,
) -> wa::message::StickerMessage {
    submessage_with_upload!(StickerMessage {}, header, up, context)
}

fn image_submessage(
    header: &OutgoingMedia,
    up: MediaUpload,
    context: buffa::MessageField<wa::ContextInfo>,
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
    descriptor: &DownloadableMedia,
) -> Result<Vec<u8>, WamuxError> {
    client
        .download_from_params(&download_params(descriptor))
        .await
        .map_err(client_err)
}

/// Build the download parameters, encrypted or not.
///
/// Issue #6: a channel's media carries NO key. Decoding a newsletter
/// `ImageMessage` shows no `media_key` and no `file_enc_sha256` at all, because
/// that media is served in the clear and authenticated by `file_sha256` alone.
/// `DownloadParams::encrypted` cannot express that, so a relay that only ever
/// called it answered for encrypted media and failed for the rest.
///
/// Absence is the proto3 default (empty bytes), the same rule the outbound side
/// uses, and the library already has the branch: `media_key: None` takes
/// `MediaDecryption::Plaintext`, which verifies `file_sha256` rather than
/// skipping verification. So this widens what the core can relay without
/// loosening what it checks.
///
/// The kind may be one of the two a received sticker pack names (issue #58);
/// SendMedia still refuses those, through `MediaKind::parse_sendable`.
fn download_params(descriptor: &DownloadableMedia) -> DownloadParams {
    let encrypted = !descriptor.media_key.is_empty();
    DownloadParams {
        direct_path: descriptor.direct_path.clone(),
        media_key: encrypted.then(|| descriptor.media_key.clone()),
        file_sha256: descriptor.file_sha256.clone(),
        // Only meaningful alongside a key: it is the hash of the ENCRYPTED bytes.
        file_enc_sha256: encrypted.then(|| descriptor.file_enc_sha256.clone()),
        file_length: descriptor.file_length,
        media_type: descriptor.kind.media_type(),
    }
}

#[cfg(test)]
#[path = "media_transfer_tests.rs"]
mod tests;
