//! Upload media for sending and download received media (lazy, from descriptor).

use wacore::download::MediaType;
use wamux_types::{DownloadableMedia, Jid, MediaKind, OutgoingMedia};
use whatsapp_rust::buffa;
use whatsapp_rust::download::{DownloadLimits, DownloadParams};
use whatsapp_rust::upload::UploadResponse;
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::waproto::whatsapp::message::{
    AudioMessage, DocumentMessage, ImageMessage, StickerMessage, VideoMessage,
};
use whatsapp_rust::{Client, SendResult};

use crate::domain::outgoing_context::outgoing_context;
use crate::domain::wire_defaults::{nonempty_bytes, nonempty_string, nonzero_u32};
use crate::error::{WamuxError, client_err};
use crate::spool::{MediaSpool, SpooledDownload, StagedUpload};

pub async fn send_media(
    client: &Client,
    to: Jid,
    media: &OutgoingMedia,
    staged: StagedUpload,
) -> Result<SendResult, WamuxError> {
    // Validated before the upload: a malformed quoted_message must refuse the
    // send before the CDN sees a byte (#230). The context is rebuilt inside
    // `build_media_message`; building it twice is cheap, and one builder keeps
    // a single decode.
    outgoing_context(&media.context)?;
    let upload = upload_staged(client, staged, media.kind.media_type()).await?;
    let message = build_media_message(media, upload.into())?;
    client
        .send_message(to.into_lib(), message)
        .await
        .map_err(client_err)
}

/// Upload the spooled ciphertext (#79). The reservation is held until the
/// upload ends, success or error, because the file occupies the disk until
/// then; the library never touches disk itself.
pub(crate) async fn upload_staged(
    client: &Client,
    staged: StagedUpload,
    media_type: MediaType,
) -> Result<UploadResponse, WamuxError> {
    let StagedUpload {
        source,
        info,
        reservation: _held,
    } = staged;
    client
        .upload_stream(source, info, media_type)
        .await
        .map_err(client_err)
}

/// The upload fields the outgoing sub-messages need, owned by wamux.
///
/// `UploadResponse` is `#[non_exhaustive]` (since whatsapp-rust 0.7) with no
/// public constructor, so it cannot be built outside the crate: the builders
/// below would be untestable if they took it directly. Naming the six
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
    /// Without it (#18) audio and voice notes draw their bubble and refuse to
    /// play, with a `delivered` receipt and no nack, even when the bytes, the
    /// container, the encryption and the descriptor are all right.
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
    let context = outgoing_context(&header.context)?;
    let kind = header.kind;
    let mut message = wa::Message::default();
    match kind {
        MediaKind::Image => {
            message.image_message =
                buffa::MessageField::some(image_submessage(header, up, context));
        }
        // PTV (video note, the round "instant video") is the SAME VideoMessage
        // wire shape, just carried in a different Message slot. The edge sets
        // header.ptv; the core only relays it into ptv_message vs video_message.
        MediaKind::Video if header.ptv => {
            message.ptv_message = buffa::MessageField::some(video_submessage(header, up, context));
        }
        MediaKind::Video => {
            message.video_message =
                buffa::MessageField::some(video_submessage(header, up, context));
        }
        MediaKind::Audio => {
            message.audio_message =
                buffa::MessageField::some(audio_submessage(header, up, context));
        }
        MediaKind::Document => {
            message.document_message =
                buffa::MessageField::some(document_submessage(header, up, context));
        }
        MediaKind::Sticker => {
            message.sticker_message =
                buffa::MessageField::some(sticker_submessage(header, up, context));
        }
        MediaKind::StickerPack | MediaKind::StickerPackThumbnail => {
            return Err(kind.refused_as_sendable());
        }
    }
    Ok(message)
}

/// The five wa media sub-messages duplicate the exact same upload/mime/context
/// field names, but the protobuf generator (buffa since 0.7) emits no shared
/// trait to abstract over them, so this macro expands the shared field
/// assignments and each builder states only its type-specific fields. The
/// structs are `#[non_exhaustive]`, so they are assigned on a default.
macro_rules! submessage_with_upload {
    ($ty:ident { $($field:ident : $value:expr),* $(,)? }, $header:expr, $up:expr, $context:expr) => {
        {
            let mut sub = $ty::default();
            sub.url = Some($up.url);
            sub.direct_path = Some($up.direct_path);
            sub.media_key = Some($up.media_key.to_vec());
            sub.file_sha256 = Some($up.file_sha256.to_vec());
            sub.file_enc_sha256 = Some($up.file_enc_sha256.to_vec());
            sub.file_length = Some($up.file_length);
            // Unconditional `Some`, matching the library's own builders: this
            // comes from the finished upload, not from the wire, so it is never
            // the proto3 default the wire-defaults rule is about.
            sub.media_key_timestamp = Some($up.media_key_timestamp);
            sub.mimetype = nonempty_string(&$header.mime_type);
            sub.context_info = $context;
            $(sub.$field = $value;)*
            sub
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

/// Lazy download from a descriptor the edge got off an inbound message event,
/// staged in the spool (#79) instead of held in memory.
///
/// Both refusals happen before anything is fetched: a descriptor larger than
/// `max_bytes`, then a budget that cannot hold it. A descriptor that declares
/// no length reserves `max_bytes`, the most a download may be. The writer
/// refuses to outgrow that, so a CDN cannot write more than was reserved.
pub async fn download(
    client: &Client,
    spool: &MediaSpool,
    descriptor: &DownloadableMedia,
    max_bytes: u64,
) -> Result<SpooledDownload, WamuxError> {
    if descriptor.file_length > max_bytes {
        return Err(WamuxError::ResourceExhausted(
            "media exceeds size limit".to_string(),
        ));
    }
    let reserve = match descriptor.file_length {
        0 => max_bytes,
        declared => declared,
    };
    let slot = spool.begin_download(reserve).await?;
    // Built after the reservation so an oversize descriptor stays
    // ResourceExhausted whatever else is wrong with it.
    let params = download_params(descriptor)?;
    // No library-side cap: the spool reservation above is the ceiling, and the
    // writer refuses to outgrow it.
    let writer = client
        .download_to_writer(&params, slot.writer, DownloadLimits::default())
        .await
        .map_err(client_err)?;
    SpooledDownload::seal(writer, slot.reservation)
}

/// Build the download parameters, encrypted or not.
///
/// A channel's media carries NO key (#6): a newsletter `ImageMessage` has no
/// `media_key` and no `file_enc_sha256` at all, because that media is served in
/// the clear and authenticated by `file_sha256` alone. An empty key therefore
/// selects `DownloadParams::plaintext`, which verifies `file_sha256` rather than
/// skipping verification, so this widens what the core can relay without
/// loosening what it checks. `file_enc_sha256` is only meaningful alongside a
/// key: it is the hash of the ENCRYPTED bytes.
///
/// The library validates the descriptor (key and hash lengths, a media type
/// that fits the chosen form) and refuses a malformed one before any fetch.
///
/// The kind may be one of the two a received sticker pack names (issue #58);
/// SendMedia still refuses those, through `MediaKind::parse_sendable`.
fn download_params(descriptor: &DownloadableMedia) -> Result<DownloadParams, WamuxError> {
    let media_type = descriptor.kind.media_type();
    let params = if descriptor.media_key.is_empty() {
        DownloadParams::plaintext(
            descriptor.direct_path.as_str(),
            &descriptor.file_sha256,
            descriptor.file_length,
            media_type,
        )
    } else {
        DownloadParams::encrypted(
            descriptor.direct_path.as_str(),
            &descriptor.media_key,
            &descriptor.file_sha256,
            &descriptor.file_enc_sha256,
            descriptor.file_length,
            media_type,
        )
    };
    // A descriptor the library refuses is a bad request, not a core that is down.
    params.map_err(|e| WamuxError::InvalidArgument(e.to_string()))
}

#[cfg(test)]
#[path = "media_transfer_tests.rs"]
mod tests;
