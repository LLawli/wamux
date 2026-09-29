//! Received sticker packs (issue #58): the archive's descriptor, the thumbnail's
//! descriptor, and the typed summary an edge draws the bubble from.
//!
//! Receive-only. Sending a pack is not relayed, so the two media types here are
//! download-only and never become `MediaKind` variants: that enum keeps the
//! upload type and the OUTGOING sub-message in lockstep, and a pack has no
//! sub-message SendMedia could build.

use wacore::download::MediaType;
use whatsapp_rust::waproto::whatsapp::message::StickerPackMessage;
use whatsapp_rust::waproto::whatsapp::message::sticker_pack_message::{Sticker, StickerPackOrigin};

use crate::proto::v1 as pb;

/// `MediaDescriptor.media_type` of a received pack's ZIP.
pub(crate) const STICKER_PACK_MEDIA_TYPE: &str = "sticker_pack";
/// `MediaDescriptor.media_type` of a received pack's thumbnail.
pub(crate) const STICKER_PACK_THUMBNAIL_MEDIA_TYPE: &str = "sticker_pack_thumbnail";

/// The library type a download-only token decrypts under, if it is one.
///
/// The type is what picks the HKDF info ("WhatsApp Sticker Pack Keys" vs
/// "... Thumbnail Keys"), so the thumbnail, which shares the pack's media key,
/// still derives its own cipher keys.
pub(crate) fn download_only_media_type(value: &str) -> Option<MediaType> {
    match value {
        STICKER_PACK_MEDIA_TYPE => Some(MediaType::StickerPack),
        STICKER_PACK_THUMBNAIL_MEDIA_TYPE => Some(MediaType::StickerPackThumbnail),
        _ => None,
    }
}

/// The pack's ZIP as a descriptor `DownloadMedia` takes unchanged.
///
/// Hand-built rather than through `media_descriptor!`: a StickerPackMessage has
/// no `mimetype` field, so `mime_type` relays as the proto3 default. Absent
/// upstream stays absent here; the core does not fill in "application/zip".
pub(crate) fn pack_descriptor(pack: &StickerPackMessage) -> pb::MediaDescriptor {
    pb::MediaDescriptor {
        direct_path: pack.direct_path.clone().unwrap_or_default(),
        media_key: pack.media_key.clone().unwrap_or_default(),
        file_enc_sha256: pack.file_enc_sha256.clone().unwrap_or_default(),
        file_sha256: pack.file_sha256.clone().unwrap_or_default(),
        file_length: pack.file_length.unwrap_or(0),
        mime_type: String::new(),
        media_type: STICKER_PACK_MEDIA_TYPE.to_string(),
    }
}

/// The thumbnail as its own descriptor, under the PACK's media key.
///
/// One key for two files is upstream's shape: the message carries a single
/// `mediaKey`, and `build_sticker_pack_message` refuses a thumbnail uploaded
/// under another. `file_length` stays 0 because the message declares none; the
/// library reads it only as a buffer hint (`download_capacity`), and the bytes
/// are still checked against both hashes and the MAC.
fn thumbnail_descriptor(pack: &StickerPackMessage) -> Option<pb::MediaDescriptor> {
    let direct_path = pack.thumbnail_direct_path.clone()?;
    Some(pb::MediaDescriptor {
        direct_path,
        media_key: pack.media_key.clone().unwrap_or_default(),
        file_enc_sha256: pack.thumbnail_enc_sha256.clone().unwrap_or_default(),
        file_sha256: pack.thumbnail_sha256.clone().unwrap_or_default(),
        file_length: 0,
        mime_type: String::new(),
        media_type: STICKER_PACK_THUMBNAIL_MEDIA_TYPE.to_string(),
    })
}

/// Everything the pack message says beyond the archive's own descriptor.
pub(crate) fn sticker_pack_info(pack: &StickerPackMessage) -> pb::StickerPackInfo {
    pb::StickerPackInfo {
        pack_id: pack.sticker_pack_id.clone().unwrap_or_default(),
        name: pack.name.clone().unwrap_or_default(),
        publisher: pack.publisher.clone().unwrap_or_default(),
        description: pack.pack_description.clone().unwrap_or_default(),
        stickers: pack.stickers.iter().map(sticker_pack_entry).collect(),
        tray_icon_file_name: pack.tray_icon_file_name.clone().unwrap_or_default(),
        pack_size: pack.sticker_pack_size.unwrap_or(0),
        origin: pack
            .sticker_pack_origin
            .map(origin_label)
            .unwrap_or_default(),
        thumbnail: thumbnail_descriptor(pack),
        thumbnail_width: pack.thumbnail_width.unwrap_or(0),
        thumbnail_height: pack.thumbnail_height.unwrap_or(0),
    }
}

fn sticker_pack_entry(sticker: &Sticker) -> pb::StickerPackEntry {
    pb::StickerPackEntry {
        file_name: sticker.file_name.clone().unwrap_or_default(),
        is_animated: sticker.is_animated.unwrap_or(false),
        emojis: sticker.emojis.clone(),
        accessibility_label: sticker.accessibility_label.clone().unwrap_or_default(),
        is_lottie: sticker.is_lottie.unwrap_or(false),
        mime_type: sticker.mimetype.clone().unwrap_or_default(),
    }
}

/// Lowercase wire tokens, the same convention as `ReceiptEvent.type`. The enum
/// is closed in the waproto, so an unknown number never reaches here: buffa's
/// decoder leaves the field `None` when `from_i32` fails, so it relays empty,
/// and the number survives only in `raw_message`.
fn origin_label(origin: StickerPackOrigin) -> String {
    match origin {
        StickerPackOrigin::FIRST_PARTY => "first_party",
        StickerPackOrigin::THIRD_PARTY => "third_party",
        StickerPackOrigin::USER_CREATED => "user_created",
    }
    .to_string()
}

#[cfg(test)]
#[path = "sticker_packs_tests.rs"]
mod tests;
