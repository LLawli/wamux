//! The kinds of media the core relays (#63, #114). One closed enum where there
//! were three: `media_transfer::MediaKind` (what SendMedia takes),
//! `status::StatusMediaKind` (what a status can be) and the two download-only
//! tokens of a received sticker pack (#58). Each operation accepts its own
//! subset through its own parser, and every wire token is written here once.

use wacore::download::MediaType;

use crate::error::WamuxError;

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
    /// Every kind, for tests and token tables.
    pub const ALL: [MediaKind; 7] = [
        Self::Image,
        Self::Video,
        Self::Audio,
        Self::Document,
        Self::Sticker,
        Self::StickerPack,
        Self::StickerPackThumbnail,
    ];

    /// What SendMedia accepts: image, video, audio, document, sticker.
    /// `InvalidArgument("unknown media_type '<v>'")` otherwise.
    pub fn parse_sendable(value: &str) -> Result<Self, WamuxError> {
        Self::ALL[..5]
            .iter()
            .copied()
            .find(|kind| kind.token() == value)
            .ok_or_else(|| unknown_media_type(value))
    }

    /// What PostStatusMedia accepts: image, video.
    /// `InvalidArgument("status media_type must be image|video, got '<v>'")` otherwise.
    pub fn parse_status(value: &str) -> Result<Self, WamuxError> {
        Self::ALL[..2]
            .iter()
            .copied()
            .find(|kind| kind.token() == value)
            .ok_or_else(|| {
                WamuxError::InvalidArgument(format!(
                    "status media_type must be image|video, got '{value}'"
                ))
            })
    }

    /// What DownloadMedia accepts: all seven.
    /// `InvalidArgument("unknown media_type '<v>'")` otherwise.
    pub fn parse_downloadable(value: &str) -> Result<Self, WamuxError> {
        Self::ALL
            .iter()
            .copied()
            .find(|kind| kind.token() == value)
            .ok_or_else(|| unknown_media_type(value))
    }

    /// The `media_type` token on the wire: `image`, ..., `sticker_pack`,
    /// `sticker_pack_thumbnail`.
    pub fn token(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Document => "document",
            Self::Sticker => "sticker",
            Self::StickerPack => "sticker_pack",
            Self::StickerPackThumbnail => "sticker_pack_thumbnail",
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

fn unknown_media_type(value: &str) -> WamuxError {
    WamuxError::InvalidArgument(format!("unknown media_type '{value}'"))
}
