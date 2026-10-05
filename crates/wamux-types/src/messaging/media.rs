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

/// The kind first (`MediaKind::parse_sendable`), then the context.
impl TryFrom<pb::SendMediaHeader> for OutgoingMedia {
    type Error = WamuxError;

    fn try_from(header: pb::SendMediaHeader) -> Result<Self, WamuxError> {
        let kind = MediaKind::parse_sendable(&header.media_type)?;
        Ok(Self {
            kind,
            context: OutgoingContext::from_proto(
                header.mentions,
                header.quote,
                header.ephemeral_seconds,
            )?,
            mime_type: header.mime_type,
            caption: header.caption,
            filename: header.filename,
            ptt: header.ptt,
            seconds: header.seconds,
            waveform: header.waveform,
            ptv: header.ptv,
        })
    }
}

/// `MediaKind::parse_downloadable`: the five sendable kinds plus a received
/// sticker pack and its thumbnail (#58).
impl TryFrom<pb::MediaDescriptor> for DownloadableMedia {
    type Error = WamuxError;

    fn try_from(descriptor: pb::MediaDescriptor) -> Result<Self, WamuxError> {
        Ok(Self {
            kind: MediaKind::parse_downloadable(&descriptor.media_type)?,
            direct_path: descriptor.direct_path,
            media_key: descriptor.media_key,
            file_enc_sha256: descriptor.file_enc_sha256,
            file_sha256: descriptor.file_sha256,
            file_length: descriptor.file_length,
            mime_type: descriptor.mime_type,
        })
    }
}
