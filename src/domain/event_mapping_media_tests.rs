//! Media-descriptor mapping tests, split out of event_mapping_tests.rs to
//! keep both files under the 500-line rule.

use super::tests::mapped_media;
use super::*;

/// The five wa media sub-messages share descriptor field names (production
/// dedupes them with `media_descriptor!`); this mirrors that table shape so
/// each kind pins type/mime/size/path/caption in one invocation.
macro_rules! media_descriptor_test {
    ($name:ident, $field:ident, $ty:ident, $kind:literal, $mime:literal, caption: $caption:literal) => {
        media_descriptor_test!(@build $name, $field, $ty, $kind, $mime,
            { caption: Some($caption.to_string()) }, $caption);
    };
    // AudioMessage/StickerMessage have no caption field in the WA proto: the
    // mapped caption must come out empty.
    ($name:ident, $field:ident, $ty:ident, $kind:literal, $mime:literal) => {
        media_descriptor_test!(@build $name, $field, $ty, $kind, $mime, {}, "");
    };
    (@build $name:ident, $field:ident, $ty:ident, $kind:literal, $mime:literal,
     { $($extra:ident : $value:expr),* }, $expected_caption:expr) => {
        #[test]
        fn $name() {
            let (media, caption) = mapped_media(wa::Message {
                $field: whatsapp_rust::buffa::MessageField::some(wa::message::$ty {
                    mimetype: Some($mime.to_string()),
                    file_length: Some(2048),
                    direct_path: Some(concat!("/v/t62.", $kind).to_string()),
                    $($extra: $value,)*
                    ..Default::default()
                }),
                ..Default::default()
            });
            assert_eq!(media.media_type, $kind);
            assert_eq!(media.mime_type, $mime);
            assert_eq!(media.file_length, 2048);
            assert_eq!(media.direct_path, concat!("/v/t62.", $kind));
            assert_eq!(caption, $expected_caption);
        }
    };
}

media_descriptor_test!(maps_image_media_descriptor_with_caption, image_message, ImageMessage, "image", "image/jpeg", caption: "a cat");
media_descriptor_test!(maps_video_media_descriptor_with_caption, video_message, VideoMessage, "video", "video/mp4", caption: "a clip");
media_descriptor_test!(
    maps_audio_media_descriptor_without_caption,
    audio_message,
    AudioMessage,
    "audio",
    "audio/ogg; codecs=opus"
);
media_descriptor_test!(maps_document_media_descriptor_with_caption, document_message, DocumentMessage, "document", "application/pdf", caption: "the invoice");
media_descriptor_test!(
    maps_sticker_media_descriptor_without_caption,
    sticker_message,
    StickerMessage,
    "sticker",
    "image/webp"
);

// Key material relays byte-for-byte; pinned once, the macro above covers the
// shape shared by all five kinds.
#[test]
fn media_descriptor_relays_key_material() {
    let (media, _) = mapped_media(wa::Message {
        image_message: whatsapp_rust::buffa::MessageField::some(wa::message::ImageMessage {
            media_key: Some(vec![1, 2, 3]),
            file_enc_sha256: Some(vec![4, 5]),
            file_sha256: Some(vec![6, 7]),
            ..Default::default()
        }),
        ..Default::default()
    });
    assert_eq!(media.media_key, [1u8, 2, 3]);
    assert_eq!(media.file_enc_sha256, [4u8, 5]);
    assert_eq!(media.file_sha256, [6u8, 7]);
}

// Issue #58: a received pack used to map to an event with no descriptor, so an
// edge could see it arrived and could not open it. Its ZIP now rides `media`,
// its caption rides `caption`, and the rest rides `sticker_pack`.
#[test]
fn a_sticker_pack_maps_to_its_archive_and_summary() {
    use wa::message::StickerPackMessage;
    let out = super::tests::mapped_inbound(wa::Message {
        sticker_pack_message: whatsapp_rust::buffa::MessageField::some(StickerPackMessage {
            name: Some("A pack".to_string()),
            caption: Some("for you".to_string()),
            direct_path: Some("/v/t62.15575-24/pack.enc".to_string()),
            file_length: Some(501_859),
            thumbnail_direct_path: Some("/v/t62.15575-24/thumb.enc".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    });
    let media = out.media.expect("a pack yields a descriptor");
    assert_eq!(media.media_type, "sticker_pack");
    assert_eq!(media.file_length, 501_859);
    assert_eq!(out.caption, "for you");
    let pack = out.sticker_pack.expect("a pack yields its summary");
    assert_eq!(pack.name, "A pack");
    assert_eq!(
        pack.thumbnail.map(|t| t.media_type).as_deref(),
        Some("sticker_pack_thumbnail")
    );
}

// Only a pack sets the summary; a single sticker keeps it unset.
#[test]
fn a_single_sticker_carries_no_pack_summary() {
    let out = super::tests::mapped_inbound(wa::Message {
        sticker_message: whatsapp_rust::buffa::MessageField::some(wa::message::StickerMessage {
            direct_path: Some("/v/t62.sticker".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    });
    assert!(out.sticker_pack.is_none());
}
