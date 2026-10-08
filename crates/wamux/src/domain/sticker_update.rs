//! The account's sticker mutations (#148, part 1 of #141): a sticker favorited,
//! unfavorited or removed from the recent list on a linked device. They
//! reached the socket as RawEvent with the library's JSON.

use wacore::types::events::{FavoriteStickerUpdate, RemoveRecentStickerUpdate};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use whatsapp_rust::waproto::whatsapp::sync_action_value::StickerAction;

use crate::proto::v1 as pb;

/// The favorite flag, the media as a `MediaDescriptor` DownloadMedia takes,
/// and the rest of the action as sent.
pub fn favorite_sticker_update_of(update: &FavoriteStickerUpdate) -> pb::StickerUpdate {
    let action = &update.action;
    let change = pb::FavoriteStickerChange {
        is_favorite: action.is_favorite,
        media: sticker_media_of(action, &update.filehash),
        url: action.url.clone(),
        width: action.width,
        height: action.height,
        is_lottie: action.is_lottie,
        is_avatar_sticker: action.is_avatar_sticker,
        image_hash: action.image_hash.clone(),
        device_id_hint: action.device_id_hint,
    };
    sticker_update_of(
        &update.filehash,
        update.timestamp.timestamp_millis(),
        update.from_full_sync,
        pb::sticker_update::Action::Favorite(change),
    )
}

/// `last_sticker_sent_ts` is already ms on the wire (measured live 2026-10-08).
pub fn remove_recent_sticker_update_of(update: &RemoveRecentStickerUpdate) -> pb::StickerUpdate {
    let change = pb::RemoveRecentStickerChange {
        last_sticker_sent_ts: update.action.last_sticker_sent_ts,
    };
    sticker_update_of(
        &update.filehash,
        update.timestamp.timestamp_millis(),
        update.from_full_sync,
        pb::sticker_update::Action::RemoveRecent(change),
    )
}

fn sticker_update_of(
    filehash: &str,
    timestamp_ms: i64,
    from_full_sync: bool,
    action: pb::sticker_update::Action,
) -> pb::StickerUpdate {
    pb::StickerUpdate {
        filehash: filehash.to_string(),
        timestamp: timestamp_ms,
        from_full_sync,
        action: Some(action),
    }
}

/// Unset when the action has no `direct_path`: without it the sticker cannot
/// be downloaded, and an empty descriptor would read as a real one.
/// `file_length` crosses as sent (a captured action carried 0).
fn sticker_media_of(action: &StickerAction, filehash: &str) -> Option<pb::MediaDescriptor> {
    let direct_path = action.direct_path.clone()?;
    Some(pb::MediaDescriptor {
        direct_path,
        media_key: action.media_key.clone().unwrap_or_default(),
        file_enc_sha256: action.file_enc_sha256.clone().unwrap_or_default(),
        file_sha256: file_sha256_of(filehash),
        file_length: action.file_length.unwrap_or_default(),
        mime_type: action.mimetype.clone().unwrap_or_default(),
        media_type: pb::MediaType::Sticker as i32,
    })
}

/// The library checks the DECRYPTED bytes against `file_sha256` (measured
/// live 2026-10-08: empty fails with "SHA-256 mismatch"), and the sticker's
/// `filehash` is the base64 of exactly that hash. Anything that does not
/// decode to 32 bytes leaves it empty; the raw filehash still crosses.
fn file_sha256_of(filehash: &str) -> Vec<u8> {
    match STANDARD.decode(filehash) {
        Ok(bytes) if bytes.len() == 32 => bytes,
        _ => Vec::new(),
    }
}

#[cfg(test)]
#[path = "sticker_update_tests.rs"]
mod tests;
