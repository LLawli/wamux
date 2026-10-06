//! `status` (#115): the tests that lived inline, moved so the loop can seal
//! them. Reading a RevokeStatus moved to `wamux-types`
//! (`messaging/status_tests.rs`), with the same assertions.

use wacore::download::MediaType;
use wamux_proto::v1 as pb;
use wamux_types::MediaKind;

use crate::error::WamuxError;

#[test]
fn status_media_kind_parses_image_and_video() {
    assert!(matches!(
        MediaKind::parse_status(pb::MediaType::Image as i32)
            .unwrap()
            .media_type(),
        MediaType::Image
    ));
    assert!(matches!(
        MediaKind::parse_status(pb::MediaType::Video as i32)
            .unwrap()
            .media_type(),
        MediaType::Video
    ));
}

// Audio/document/sticker are valid for a normal message but NOT for a
// status; the edge sending one is an InvalidArgument, not a silent fallback.
#[test]
fn status_media_kind_rejects_non_status_kinds() {
    let refused = [
        pb::MediaType::Audio,
        pb::MediaType::Document,
        pb::MediaType::Sticker,
        pb::MediaType::Unspecified,
    ];
    for bad in refused {
        assert!(
            matches!(
                MediaKind::parse_status(bad as i32),
                Err(WamuxError::InvalidArgument(_))
            ),
            "expected {bad:?} to be rejected"
        );
    }
}
