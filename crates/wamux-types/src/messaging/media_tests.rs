//! `OutgoingMedia`, `DownloadableMedia` (#115).

use wamux_proto::v1 as pb;

use crate::{DownloadableMedia, Jid, MediaKind, OutgoingMedia, WamuxError};

fn invalid_argument<T: std::fmt::Debug>(result: Result<T, WamuxError>) -> String {
    match result {
        Err(WamuxError::InvalidArgument(message)) => message,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

fn header(media_type: &str) -> pb::SendMediaHeader {
    pb::SendMediaHeader {
        media_type: media_type.to_string(),
        ..Default::default()
    }
}

fn descriptor(media_type: &str) -> pb::MediaDescriptor {
    pb::MediaDescriptor {
        direct_path: "/v/t62.7118-24/enc".to_string(),
        media_key: vec![1; 32],
        file_enc_sha256: vec![2; 32],
        file_sha256: vec![3; 32],
        file_length: 2048,
        mime_type: "image/jpeg".to_string(),
        media_type: media_type.to_string(),
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
        media_type: "audio".to_string(),
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
    for token in ["sticker_pack", "sticker_pack_thumbnail", "gif", ""] {
        assert_eq!(
            invalid_argument(OutgoingMedia::try_from(header(token))),
            format!("unknown media_type '{token}'")
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
        ..header("gif")
    };
    assert_eq!(
        invalid_argument(OutgoingMedia::try_from(request)),
        "unknown media_type 'gif'"
    );
}

#[test]
fn downloadable_media_copies_every_field() {
    assert_eq!(
        DownloadableMedia::try_from(descriptor("image")).unwrap(),
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
fn downloadable_media_accepts_the_pack_tokens() {
    let kinds = ["sticker_pack", "sticker_pack_thumbnail", "sticker"]
        .map(|token| DownloadableMedia::try_from(descriptor(token)).unwrap().kind);
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
    assert_eq!(
        invalid_argument(DownloadableMedia::try_from(descriptor("gif"))),
        "unknown media_type 'gif'"
    );
}
