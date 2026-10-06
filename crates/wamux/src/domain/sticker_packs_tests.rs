//! Unit tests for `sticker_packs`, split out to keep the module small.
//!
//! The fixture copies the SHAPE of the pack #58 was measured on (field set,
//! lengths, origin) and none of its secrets: the real message carries a
//! contact's media key, so key, hashes and paths here are synthetic.

use super::*;
use whatsapp_rust::waproto::whatsapp::message::sticker_pack_message::Sticker;

fn sticker(file_name: &str, animated: bool) -> Sticker {
    Sticker {
        file_name: Some(file_name.to_string()),
        is_animated: Some(animated),
        emojis: vec!["😀".to_string(), "🎉".to_string()],
        mimetype: Some("image/webp".to_string()),
        ..Default::default()
    }
}

fn pack() -> StickerPackMessage {
    StickerPackMessage {
        sticker_pack_id: Some("maker-app-pack-1".to_string()),
        name: Some("A pack".to_string()),
        publisher: Some("Someone".to_string()),
        stickers: vec![sticker("01_aa.webp", false), sticker("02_bb.webp", true)],
        file_length: Some(501_859),
        file_sha256: Some(vec![6u8; 32]),
        file_enc_sha256: Some(vec![7u8; 32]),
        media_key: Some(vec![8u8; 32]),
        direct_path: Some("/v/t62.15575-24/pack.enc".to_string()),
        tray_icon_file_name: Some("maker-app-pack-1.png".to_string()),
        thumbnail_direct_path: Some("/v/t62.15575-24/thumb.enc".to_string()),
        thumbnail_sha256: Some(vec![16u8; 32]),
        thumbnail_enc_sha256: Some(vec![17u8; 32]),
        thumbnail_height: Some(252),
        thumbnail_width: Some(252),
        sticker_pack_size: Some(501_503),
        sticker_pack_origin: Some(StickerPackOrigin::THIRD_PARTY),
        ..Default::default()
    }
}

// Each token decrypts under its own HKDF info: the thumbnail shares the pack's
// key, so a thumbnail downloaded as a pack would fail its MAC.
#[test]
fn each_download_only_token_picks_its_own_hkdf_info() {
    let pack = MediaKind::StickerPack.media_type();
    assert_eq!(pack.app_info(), "WhatsApp Sticker Pack Keys");
    assert_eq!(pack.upload_path(), "/mms/sticker-pack");
    let thumb = MediaKind::StickerPackThumbnail.media_type();
    assert_eq!(thumb.app_info(), "WhatsApp Sticker Pack Thumbnail Keys");
    assert_eq!(thumb.upload_path(), "/mms/thumbnail-sticker-pack");
}

// Exact literals, like `MediaKind::parse_downloadable`: the library's own
// "sticker-pack" spelling is its MMS path segment, not this contract's token.
#[test]
fn other_spellings_are_not_download_only_tokens() {
    for value in ["sticker-pack", "Sticker_Pack", ""] {
        assert!(
            MediaKind::parse_downloadable(value).is_err(),
            "value: {value:?}"
        );
    }
}

#[test]
fn the_pack_descriptor_is_the_archive_verbatim() {
    let descriptor = pack_descriptor(&pack());
    assert_eq!(descriptor.media_type, "sticker_pack");
    assert_eq!(descriptor.direct_path, "/v/t62.15575-24/pack.enc");
    assert_eq!(descriptor.media_key, vec![8u8; 32]);
    assert_eq!(descriptor.file_sha256, vec![6u8; 32]);
    assert_eq!(descriptor.file_enc_sha256, vec![7u8; 32]);
    assert_eq!(descriptor.file_length, 501_859);
    // The message has no mimetype field; the core does not invent one.
    assert_eq!(descriptor.mime_type, "");
}

// The thumbnail is its own file: its own path and hashes, the PACK's key.
#[test]
fn the_thumbnail_descriptor_pairs_its_own_hashes_with_the_pack_key() {
    let thumb = sticker_pack_info(&pack())
        .thumbnail
        .expect("thumbnail descriptor");
    assert_eq!(thumb.media_type, "sticker_pack_thumbnail");
    assert_eq!(thumb.direct_path, "/v/t62.15575-24/thumb.enc");
    assert_eq!(thumb.media_key, vec![8u8; 32]);
    assert_eq!(thumb.file_sha256, vec![16u8; 32]);
    assert_eq!(thumb.file_enc_sha256, vec![17u8; 32]);
    assert_eq!(thumb.file_length, 0);
}

#[test]
fn no_thumbnail_path_means_no_thumbnail_descriptor() {
    let info = sticker_pack_info(&StickerPackMessage {
        thumbnail_direct_path: None,
        ..pack()
    });
    assert!(info.thumbnail.is_none());
}

#[test]
fn the_summary_relays_the_pack_and_each_entry() {
    let info = sticker_pack_info(&pack());
    assert_eq!(info.pack_id, "maker-app-pack-1");
    assert_eq!(info.name, "A pack");
    assert_eq!(info.publisher, "Someone");
    assert_eq!(info.tray_icon_file_name, "maker-app-pack-1.png");
    assert_eq!(info.pack_size, 501_503);
    assert_eq!(info.origin(), pb::StickerPackOrigin::ThirdParty);
    assert_eq!((info.thumbnail_width, info.thumbnail_height), (252, 252));
    let names: Vec<&str> = info.stickers.iter().map(|s| s.file_name.as_str()).collect();
    assert_eq!(names, ["01_aa.webp", "02_bb.webp"]);
    assert!(!info.stickers[0].is_animated);
    assert!(info.stickers[1].is_animated);
    assert_eq!(info.stickers[1].emojis, ["😀", "🎉"]);
    assert_eq!(info.stickers[1].mime_type, "image/webp");
}

// #126: the contract's enum, one value per waproto value (was a lowercase token).
#[test]
fn each_origin_relays_as_its_enum_value() {
    let cases = [
        (
            StickerPackOrigin::FIRST_PARTY,
            pb::StickerPackOrigin::FirstParty,
        ),
        (
            StickerPackOrigin::THIRD_PARTY,
            pb::StickerPackOrigin::ThirdParty,
        ),
        (
            StickerPackOrigin::USER_CREATED,
            pb::StickerPackOrigin::UserCreated,
        ),
    ];
    for (origin, wire) in cases {
        let info = sticker_pack_info(&StickerPackMessage {
            sticker_pack_origin: Some(origin),
            ..pack()
        });
        assert_eq!(info.origin(), wire);
    }
}

// FIRST_PARTY is waproto's 0, so absence must not collapse into it: it is
// UNSPECIFIED (#126; it was "").
#[test]
fn an_absent_origin_relays_unspecified_not_first_party() {
    let info = sticker_pack_info(&StickerPackMessage {
        sticker_pack_origin: None,
        ..pack()
    });
    assert_eq!(info.origin(), pb::StickerPackOrigin::Unspecified);
}
