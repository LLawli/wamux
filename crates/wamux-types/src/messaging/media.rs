//! Media going out, and media to fetch (#115).

use wamux_proto::v1 as pb;

use crate::error::WamuxError;
use crate::media_kind::MediaKind;
use crate::messaging::key::OutgoingContext;

/// The content of a SendMedia header: its routing (`account`, `to`) is the
/// service's. Strings and numbers relay verbatim; proto3 defaults are the
/// absent field, which the domain's builder decides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutgoingMedia {
    pub kind: MediaKind,
    pub mime_type: String,
    pub caption: String,
    pub filename: String,
    pub ptt: bool,
    pub seconds: u32,
    pub waveform: Vec<u8>,
    pub ptv: bool,
    pub context: OutgoingContext,
    /// The clip of a sound sticker (#278); `Some` only on a STICKER.
    pub sticker_audio: Option<StickerAudio>,
}

/// The clip a sound sticker carries (#278), relayed verbatim as the sticker's
/// AudioMessage. The bytes stream separately; this is what describes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StickerAudio {
    pub mime_type: String,
    pub seconds: u32,
}

/// The header's `sticker_audio`, refused on any kind but a sticker (#278). It
/// reads the header alone, so the service can refuse before the first byte,
/// and the conversion below reads it the same way.
pub fn sticker_audio_of(
    kind: MediaKind,
    header: &pb::SendMediaHeader,
) -> Result<Option<StickerAudio>, WamuxError> {
    let Some(clip) = header.sticker_audio.as_ref() else {
        return Ok(None);
    };
    if kind != MediaKind::Sticker {
        return Err(WamuxError::InvalidArgument(format!(
            "sticker_audio needs media_type MEDIA_TYPE_STICKER, got {}",
            kind.to_wire().as_str_name()
        )));
    }
    Ok(Some(StickerAudio {
        mime_type: clip.mime_type.clone(),
        seconds: clip.seconds,
    }))
}

/// Where a received message's media lives, and how to open it. Empty
/// `media_key` is a channel's media, served in the clear (#6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadableMedia {
    pub kind: MediaKind,
    pub direct_path: String,
    pub media_key: Vec<u8>,
    pub file_enc_sha256: Vec<u8>,
    pub file_sha256: Vec<u8>,
    pub file_length: u64,
    pub mime_type: String,
}

/// The kind first (`MediaKind::parse_sendable`), then `sticker_audio`, then the
/// context.
impl TryFrom<pb::SendMediaHeader> for OutgoingMedia {
    type Error = WamuxError;

    fn try_from(header: pb::SendMediaHeader) -> Result<Self, WamuxError> {
        let kind = MediaKind::parse_sendable(header.media_type)?;
        let sticker_audio = sticker_audio_of(kind, &header)?;
        Ok(Self {
            kind,
            context: OutgoingContext::from_proto(
                header.mentions,
                header.quote,
                header.quoted_message,
                header.ephemeral_seconds,
                header.to.as_ref(),
            )?,
            mime_type: header.mime_type,
            caption: header.caption,
            filename: header.filename,
            ptt: header.ptt,
            seconds: header.seconds,
            waveform: header.waveform,
            ptv: header.ptv,
            sticker_audio,
        })
    }
}

/// `MediaKind::parse_downloadable`: the five sendable kinds plus a received
/// sticker pack and its thumbnail (#58).
impl TryFrom<pb::MediaDescriptor> for DownloadableMedia {
    type Error = WamuxError;

    fn try_from(descriptor: pb::MediaDescriptor) -> Result<Self, WamuxError> {
        Ok(Self {
            kind: MediaKind::parse_downloadable(descriptor.media_type)?,
            direct_path: descriptor.direct_path,
            media_key: descriptor.media_key,
            file_enc_sha256: descriptor.file_enc_sha256,
            file_sha256: descriptor.file_sha256,
            file_length: descriptor.file_length,
            mime_type: descriptor.mime_type,
        })
    }
}
