//! The kinds of media the core relays (#63, #114). One closed enum where there
//! were three: `media_transfer::MediaKind` (what SendMedia takes),
//! `status::StatusMediaKind` (what a status can be) and the two download-only
//! kinds of a received sticker pack (#58). Each operation accepts its own
//! subset through its own parser, and every wire value is written here once.

use wacore::download::MediaType;
use wamux_proto::v1 as pb;

use crate::error::WamuxError;
use crate::request_enum::{refusal, shown_value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Document,
    Sticker,
    /// A received pack's ZIP: download only (#58).
    StickerPack,
    /// A received pack's thumbnail: download only (#58).
    StickerPackThumbnail,
}

impl MediaKind {
    /// Every kind, for tests and value tables.
    pub const ALL: [MediaKind; 7] = [
        Self::Image,
        Self::Video,
        Self::Audio,
        Self::Document,
        Self::Sticker,
        Self::StickerPack,
        Self::StickerPackThumbnail,
    ];

    /// What SendMedia accepts: IMAGE, VIDEO, AUDIO, DOCUMENT, STICKER (#127).
    /// `InvalidArgument("media_type must be one of MEDIA_TYPE_IMAGE|...|MEDIA_TYPE_STICKER, got <v>")`
    /// otherwise, `<v>` the value's proto name or the bare number.
    pub fn parse_sendable(value: i32) -> Result<Self, WamuxError> {
        parse_within(value, MEDIA_TYPE_FIELD, &Self::ALL[..5])
    }

    /// What PostStatusMedia accepts: IMAGE, VIDEO (#127).
    /// `InvalidArgument("status media_type must be one of MEDIA_TYPE_IMAGE|MEDIA_TYPE_VIDEO, got <v>")`
    /// otherwise.
    pub fn parse_status(value: i32) -> Result<Self, WamuxError> {
        parse_within(value, STATUS_FIELD, &Self::ALL[..2])
    }

    /// What DownloadMedia accepts: all seven (#127).
    /// `InvalidArgument("media_type must be one of <the seven>, got <v>")` otherwise.
    pub fn parse_downloadable(value: i32) -> Result<Self, WamuxError> {
        parse_within(value, MEDIA_TYPE_FIELD, &Self::ALL)
    }

    /// The refusal for a kind the sender cannot send, in the shape the parser
    /// uses. For the defensive arm of a domain match (#127).
    pub fn refused_as_sendable(self) -> WamuxError {
        refuse(
            MEDIA_TYPE_FIELD,
            &Self::ALL[..5],
            self.to_wire().as_str_name(),
        )
    }

    /// The refusal for a kind a status cannot carry (#127).
    pub fn refused_as_status(self) -> WamuxError {
        refuse(STATUS_FIELD, &Self::ALL[..2], self.to_wire().as_str_name())
    }

    /// The `media_type` value on the wire (#127).
    pub fn to_wire(self) -> pb::MediaType {
        match self {
            Self::Image => pb::MediaType::Image,
            Self::Video => pb::MediaType::Video,
            Self::Audio => pb::MediaType::Audio,
            Self::Document => pb::MediaType::Document,
            Self::Sticker => pb::MediaType::Sticker,
            Self::StickerPack => pb::MediaType::StickerPack,
            Self::StickerPackThumbnail => pb::MediaType::StickerPackThumbnail,
        }
    }

    /// The library type it uploads and downloads under. The type picks the
    /// HKDF info, so the pack's thumbnail derives its own keys (#58).
    pub fn media_type(self) -> MediaType {
        match self {
            Self::Image => MediaType::Image,
            Self::Video => MediaType::Video,
            Self::Audio => MediaType::Audio,
            Self::Document => MediaType::Document,
            Self::Sticker => MediaType::Sticker,
            Self::StickerPack => MediaType::StickerPack,
            Self::StickerPackThumbnail => MediaType::StickerPackThumbnail,
        }
    }
}

const MEDIA_TYPE_FIELD: &str = "media_type";
const STATUS_FIELD: &str = "status media_type";

/// Every variant is listed, with no wildcard arm, so a variant added to the
/// proto breaks the build here instead of being refused silently (#127).
fn from_wire(value: i32) -> Option<MediaKind> {
    match pb::MediaType::try_from(value) {
        Ok(pb::MediaType::Image) => Some(MediaKind::Image),
        Ok(pb::MediaType::Video) => Some(MediaKind::Video),
        Ok(pb::MediaType::Audio) => Some(MediaKind::Audio),
        Ok(pb::MediaType::Document) => Some(MediaKind::Document),
        Ok(pb::MediaType::Sticker) => Some(MediaKind::Sticker),
        Ok(pb::MediaType::StickerPack) => Some(MediaKind::StickerPack),
        Ok(pb::MediaType::StickerPackThumbnail) => Some(MediaKind::StickerPackThumbnail),
        Ok(pb::MediaType::Unspecified | pb::MediaType::Unknown) | Err(_) => None,
    }
}

fn shown(value: i32) -> String {
    let name = pb::MediaType::try_from(value)
        .ok()
        .map(|wire| wire.as_str_name());
    shown_value(name, value)
}

fn refuse(field: &str, subset: &[MediaKind], got: &str) -> WamuxError {
    let names = subset.iter().map(|kind| kind.to_wire().as_str_name());
    refusal(field, names, got)
}

fn parse_within(value: i32, field: &str, subset: &[MediaKind]) -> Result<MediaKind, WamuxError> {
    from_wire(value)
        .filter(|kind| subset.contains(kind))
        .ok_or_else(|| refuse(field, subset, &shown(value)))
}
