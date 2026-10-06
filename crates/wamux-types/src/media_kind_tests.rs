//! `MediaKind` (#114, #127): one enum, three accepted subsets, one wire table.
//! Every known value is accepted by name, and every way to be wrong (UNSPECIFIED,
//! UNKNOWN, a number outside the enum, a value outside the subset) is refused
//! with the same message shape.

use wacore::download::MediaType;
use wamux_proto::v1 as pb;

use crate::{MediaKind, WamuxError};

/// The wire values, as `event_mapping`, `sticker_packs`, SendMedia,
/// PostStatusMedia and DownloadMedia write and read them.
const WIRE: [(MediaKind, pb::MediaType); 7] = [
    (MediaKind::Image, pb::MediaType::Image),
    (MediaKind::Video, pb::MediaType::Video),
    (MediaKind::Audio, pb::MediaType::Audio),
    (MediaKind::Document, pb::MediaType::Document),
    (MediaKind::Sticker, pb::MediaType::Sticker),
    (MediaKind::StickerPack, pb::MediaType::StickerPack),
    (
        MediaKind::StickerPackThumbnail,
        pb::MediaType::StickerPackThumbnail,
    ),
];

const SENDABLE: &str = "media_type must be one of MEDIA_TYPE_IMAGE|MEDIA_TYPE_VIDEO|\
MEDIA_TYPE_AUDIO|MEDIA_TYPE_DOCUMENT|MEDIA_TYPE_STICKER, got";

const STATUS: &str = "status media_type must be one of MEDIA_TYPE_IMAGE|MEDIA_TYPE_VIDEO, got";

const DOWNLOADABLE: &str = "media_type must be one of MEDIA_TYPE_IMAGE|MEDIA_TYPE_VIDEO|\
MEDIA_TYPE_AUDIO|MEDIA_TYPE_DOCUMENT|MEDIA_TYPE_STICKER|MEDIA_TYPE_STICKER_PACK|\
MEDIA_TYPE_STICKER_PACK_THUMBNAIL, got";

/// Values no parser takes: unset, unknown, and numbers the enum does not name.
const NEVER: [(i32, &str); 4] = [
    (0, "MEDIA_TYPE_UNSPECIFIED"),
    (1, "MEDIA_TYPE_UNKNOWN"),
    (42, "42"),
    (-1, "-1"),
];

fn invalid_argument(result: Result<MediaKind, WamuxError>) -> String {
    match result {
        Err(WamuxError::InvalidArgument(message)) => message,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

#[test]
fn every_kind_has_its_wire_value() {
    for (kind, wire) in WIRE {
        assert_eq!(kind.to_wire(), wire, "{kind:?}");
    }
    assert_eq!(MediaKind::ALL.len(), WIRE.len());
}

#[test]
fn download_accepts_all_seven_by_wire_value() {
    for (kind, wire) in WIRE {
        assert_eq!(MediaKind::parse_downloadable(wire as i32).unwrap(), kind);
    }
}

#[test]
fn download_refuses_unset_unknown_and_unlisted_values() {
    for (value, shown) in NEVER {
        assert_eq!(
            invalid_argument(MediaKind::parse_downloadable(value)),
            format!("{DOWNLOADABLE} {shown}")
        );
    }
}

#[test]
fn send_accepts_the_five_sendable_kinds_only() {
    for (kind, wire) in &WIRE[..5] {
        assert_eq!(MediaKind::parse_sendable(*wire as i32).unwrap(), *kind);
    }
    let download_only = [
        (pb::MediaType::StickerPack as i32, "MEDIA_TYPE_STICKER_PACK"),
        (
            pb::MediaType::StickerPackThumbnail as i32,
            "MEDIA_TYPE_STICKER_PACK_THUMBNAIL",
        ),
    ];
    for (value, shown) in download_only.into_iter().chain(NEVER) {
        assert_eq!(
            invalid_argument(MediaKind::parse_sendable(value)),
            format!("{SENDABLE} {shown}")
        );
    }
}

#[test]
fn a_status_is_an_image_or_a_video() {
    assert_eq!(
        MediaKind::parse_status(pb::MediaType::Image as i32).unwrap(),
        MediaKind::Image
    );
    assert_eq!(
        MediaKind::parse_status(pb::MediaType::Video as i32).unwrap(),
        MediaKind::Video
    );
    let not_a_status = WIRE[2..]
        .iter()
        .map(|(_, wire)| (*wire as i32, wire.as_str_name()));
    for (value, shown) in not_a_status.chain(NEVER) {
        assert_eq!(
            invalid_argument(MediaKind::parse_status(value)),
            format!("{STATUS} {shown}")
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
    for ((kind, _), media_type) in WIRE.into_iter().zip(expected) {
        assert_eq!(kind.media_type(), media_type, "{kind:?}");
    }
}

// DownloadMedia takes the descriptor back verbatim (#127): what the core writes
// on an event is what it reads on the download, for every kind.
#[test]
fn every_kind_round_trips_through_the_descriptor_value() {
    for kind in MediaKind::ALL {
        let written = kind.to_wire() as i32;
        assert_eq!(MediaKind::parse_downloadable(written).unwrap(), kind);
    }
}
