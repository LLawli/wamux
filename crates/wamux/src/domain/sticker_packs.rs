//! Received sticker packs (issue #58): the archive's descriptor, the thumbnail's
//! descriptor, and the typed summary an edge draws the bubble from.
//!
//! Receive-only. Sending a pack is not relayed, so the two media kinds here are
//! download-only: `MediaKind::parse_sendable` refuses them and
//! `build_media_message` has no outgoing sub-message for them. Their wire
//! values and library types live in `wamux_types::MediaKind` (#114).

use wamux_types::MediaKind;
use whatsapp_rust::waproto::whatsapp::message::StickerPackMessage;
use whatsapp_rust::waproto::whatsapp::message::sticker_pack_message::{Sticker, StickerPackOrigin};

use crate::proto::v1 as pb;

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
        media_type: MediaKind::StickerPack.to_wire() as i32,
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
        media_type: MediaKind::StickerPackThumbnail.to_wire() as i32,
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
            .map(origin_of)
            .unwrap_or(pb::StickerPackOrigin::Unspecified) as i32,
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

/// The enum is closed in the waproto, so an unknown number never reaches here:
/// buffa's decoder leaves the field `None` when `from_i32` fails, so it relays
/// UNSPECIFIED, and the number survives only in `raw_message`. The match lives
/// here because wacore does not re-export the waproto (#126).
fn origin_of(origin: StickerPackOrigin) -> pb::StickerPackOrigin {
    match origin {
        StickerPackOrigin::FIRST_PARTY => pb::StickerPackOrigin::FirstParty,
        StickerPackOrigin::THIRD_PARTY => pb::StickerPackOrigin::ThirdParty,
        StickerPackOrigin::USER_CREATED => pb::StickerPackOrigin::UserCreated,
    }
}

#[cfg(test)]
#[path = "sticker_packs_tests.rs"]
mod tests;
