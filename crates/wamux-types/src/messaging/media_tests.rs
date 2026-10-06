//! `OutgoingMedia`, `DownloadableMedia` (#115).

use wamux_proto::v1 as pb;

use crate::{DownloadableMedia, Jid, MediaKind, OutgoingMedia, WamuxError};

fn invalid_argument<T: std::fmt::Debug>(result: Result<T, WamuxError>) -> String {
    match result {
        Err(WamuxError::InvalidArgument(message)) => message,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

const SENDABLE: &str = "media_type must be one of MEDIA_TYPE_IMAGE|MEDIA_TYPE_VIDEO|\
MEDIA_TYPE_AUDIO|MEDIA_TYPE_DOCUMENT|MEDIA_TYPE_STICKER, got";

fn header(media_type: i32) -> pb::SendMediaHeader {
    pb::SendMediaHeader {
        media_type,
        ..Default::default()
    }
}

fn descriptor(media_type: pb::MediaType) -> pb::MediaDescriptor {
    pb::MediaDescriptor {
        direct_path: "/v/t62.7118-24/enc".to_string(),
        media_key: vec![1; 32],
        file_enc_sha256: vec![2; 32],
        file_sha256: vec![3; 32],
        file_length: 2048,
        mime_type: "image/jpeg".to_string(),
        media_type: media_type as i32,
    }
}

#[test]
fn outgoing_media_parses_its_kind_and_context() {
    let media = OutgoingMedia::try_from(pb::SendMediaHeader {
        account: None,
        to: None,
        mime_type: "audio/ogg; codecs=opus".to_string(),
        caption: "legenda".to_string(),
        mentions: vec![pb::Mention {
            jid: Some(pb::Jid {
                value: "5511888888888@s.whatsapp.net".to_string(),
            }),
        }],
        quote: None,
        media_type: pb::MediaType::Audio as i32,
        filename: "nota.ogg".to_string(),
        ptt: true,
        seconds: 7,
        waveform: vec![0, 50, 100],
        ephemeral_seconds: 90,
        ptv: true,
    })
    .unwrap();
    assert_eq!(media.kind, MediaKind::Audio);
    assert_eq!(media.mime_type, "audio/ogg; codecs=opus");
    assert_eq!(media.caption, "legenda");
    assert_eq!(media.filename, "nota.ogg");
    assert!(media.ptt);
    assert_eq!(media.seconds, 7);
    assert_eq!(media.waveform, vec![0, 50, 100]);
    assert!(media.ptv);
    assert_eq!(
        media.context.mentions,
        vec![Jid::parse("5511888888888@s.whatsapp.net").unwrap()]
    );
    assert_eq!(media.context.ephemeral_seconds, 90);
}

#[test]
fn outgoing_media_refuses_a_download_only_kind() {
    let refused = [
        (pb::MediaType::StickerPack as i32, "MEDIA_TYPE_STICKER_PACK"),
        (
            pb::MediaType::StickerPackThumbnail as i32,
            "MEDIA_TYPE_STICKER_PACK_THUMBNAIL",
        ),
        (pb::MediaType::Unspecified as i32, "MEDIA_TYPE_UNSPECIFIED"),
        (pb::MediaType::Unknown as i32, "MEDIA_TYPE_UNKNOWN"),
        (42, "42"),
    ];
    for (value, shown) in refused {
        assert_eq!(
            invalid_argument(OutgoingMedia::try_from(header(value))),
            format!("{SENDABLE} {shown}")
        );
    }
}

// The kind is read before the context, as the domain read it first.
#[test]
fn outgoing_media_checks_the_kind_before_the_context() {
    let request = pb::SendMediaHeader {
        mentions: vec![pb::Mention {
            jid: Some(pb::Jid {
                value: "not a jid".to_string(),
            }),
        }],
        ..header(42)
    };
    assert_eq!(
        invalid_argument(OutgoingMedia::try_from(request)),
        format!("{SENDABLE} 42")
    );
}

#[test]
fn downloadable_media_copies_every_field() {
    assert_eq!(
        DownloadableMedia::try_from(descriptor(pb::MediaType::Image)).unwrap(),
        DownloadableMedia {
            kind: MediaKind::Image,
            direct_path: "/v/t62.7118-24/enc".to_string(),
            media_key: vec![1; 32],
            file_enc_sha256: vec![2; 32],
            file_sha256: vec![3; 32],
            file_length: 2048,
            mime_type: "image/jpeg".to_string(),
        }
    );
}

// Issue #58: DownloadMedia takes a received pack and its thumbnail.
#[test]
fn downloadable_media_accepts_the_pack_values() {
    let kinds = [
        pb::MediaType::StickerPack,
        pb::MediaType::StickerPackThumbnail,
        pb::MediaType::Sticker,
    ]
    .map(|value| DownloadableMedia::try_from(descriptor(value)).unwrap().kind);
    assert_eq!(
        kinds,
        [
            MediaKind::StickerPack,
            MediaKind::StickerPackThumbnail,
            MediaKind::Sticker
        ]
    );
}

#[test]
fn downloadable_media_refuses_an_unknown_kind() {
    let unset = DownloadableMedia::try_from(descriptor(pb::MediaType::Unspecified));
    assert_eq!(
        invalid_argument(unset),
        "media_type must be one of MEDIA_TYPE_IMAGE|MEDIA_TYPE_VIDEO|MEDIA_TYPE_AUDIO|\
MEDIA_TYPE_DOCUMENT|MEDIA_TYPE_STICKER|MEDIA_TYPE_STICKER_PACK|\
MEDIA_TYPE_STICKER_PACK_THUMBNAIL, got MEDIA_TYPE_UNSPECIFIED"
    );
}
