//! `MediaKind` (#114): one enum, three accepted subsets, one token table.

use wacore::download::MediaType;

use crate::{MediaKind, WamuxError};

/// The wire tokens, as `event_mapping`, `sticker_packs`, SendMedia,
/// PostStatusMedia and DownloadMedia spell them today.
const TOKENS: [(MediaKind, &str); 7] = [
    (MediaKind::Image, "image"),
    (MediaKind::Video, "video"),
    (MediaKind::Audio, "audio"),
    (MediaKind::Document, "document"),
    (MediaKind::Sticker, "sticker"),
    (MediaKind::StickerPack, "sticker_pack"),
    (MediaKind::StickerPackThumbnail, "sticker_pack_thumbnail"),
];

fn invalid_argument(result: Result<MediaKind, WamuxError>) -> String {
    match result {
        Err(WamuxError::InvalidArgument(message)) => message,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

#[test]
fn every_kind_has_its_token() {
    for (kind, token) in TOKENS {
        assert_eq!(kind.token(), token);
    }
    assert_eq!(MediaKind::ALL.len(), TOKENS.len());
}

#[test]
fn download_accepts_all_seven_by_token() {
    for (kind, token) in TOKENS {
        assert_eq!(MediaKind::parse_downloadable(token).unwrap(), kind);
    }
    assert_eq!(
        invalid_argument(MediaKind::parse_downloadable("gif")),
        "unknown media_type 'gif'"
    );
}

#[test]
fn send_accepts_the_five_sendable_kinds_only() {
    for (kind, token) in &TOKENS[..5] {
        assert_eq!(MediaKind::parse_sendable(token).unwrap(), *kind);
    }
    for token in ["sticker_pack", "sticker_pack_thumbnail", "IMAGE", ""] {
        assert_eq!(
            invalid_argument(MediaKind::parse_sendable(token)),
            format!("unknown media_type '{token}'")
        );
    }
}

#[test]
fn a_status_is_an_image_or_a_video() {
    assert_eq!(MediaKind::parse_status("image").unwrap(), MediaKind::Image);
    assert_eq!(MediaKind::parse_status("video").unwrap(), MediaKind::Video);
    for token in ["audio", "document", "sticker", "sticker_pack", "x"] {
        assert_eq!(
            invalid_argument(MediaKind::parse_status(token)),
            format!("status media_type must be image|video, got '{token}'")
        );
    }
}

#[test]
fn each_kind_uploads_and_downloads_under_its_library_type() {
    let expected = [
        MediaType::Image,
        MediaType::Video,
        MediaType::Audio,
        MediaType::Document,
        MediaType::Sticker,
        MediaType::StickerPack,
        MediaType::StickerPackThumbnail,
    ];
    for ((kind, _), media_type) in TOKENS.into_iter().zip(expected) {
        assert_eq!(kind.media_type(), media_type, "{kind:?}");
    }
}
