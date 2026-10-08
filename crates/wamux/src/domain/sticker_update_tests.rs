//! #148 (part 1 of #141): sticker mutations, routed through `map_event`, so
//! each test also proves the event no longer falls into the RawEvent
//! catch-all. Values are the ones the user's phone sent on 2026-10-08 14:45Z
//! (read off the production subscription); the media key, the encrypted hash
//! and the path are fictitious, the rest as sent.

#![allow(clippy::field_reassign_with_default)] // the generated actions are built by assignment

use wacore::types::events::{Event, FavoriteStickerUpdate, RemoveRecentStickerUpdate};
use whatsapp_rust::waproto::whatsapp::sync_action_value::{
    RemoveRecentStickerAction, StickerAction,
};

use crate::domain::event_mapping::map_event;
use crate::proto::v1 as pb;
use crate::proto::v1::event_envelope::Event as PbEvent;
use crate::proto::v1::sticker_update::Action;

const FAVORITE_HASH: &str = "PJdBWV8W7cZPkk/1HuPi5ESzx4xsDn//uujshBPSRh4=";
/// `FAVORITE_HASH` decoded: what DownloadMedia checks the decrypted bytes
/// against (measured live: empty is refused, this succeeds).
const FAVORITE_SHA256: [u8; 32] = [
    0x3c, 0x97, 0x41, 0x59, 0x5f, 0x16, 0xed, 0xc6, 0x4f, 0x92, 0x4f, 0xf5, 0x1e, 0xe3, 0xe2, 0xe4,
    0x44, 0xb3, 0xc7, 0x8c, 0x6c, 0x0e, 0x7f, 0xff, 0xba, 0xe8, 0xec, 0x84, 0x13, 0xd2, 0x46, 0x1e,
];
const RECENT_HASH: &str = "OjZevFecGHv6pBDONcyCStTIkY64pf9hkxHE0aZlKTE=";
const DIRECT_PATH: &str = "/v/t62.15575-24/00000000_0000000000000148_n.enc";

/// The instant `ms` after the epoch, as the library's builders take it. A
/// macro because wamux does not depend on chrono and cannot name the type.
macro_rules! at_ms {
    ($ms:expr) => {
        wacore::time::from_millis($ms).expect("test instant")
    };
}

/// The one StickerUpdate the event maps to; anything else is a failure.
fn sticker_of(event: Event) -> pb::StickerUpdate {
    match map_event(&event).as_slice() {
        [PbEvent::Sticker(update)] => update.clone(),
        other => panic!("expected one StickerUpdate, got {other:?}"),
    }
}

/// The captured action: a 64x64 webp, no url, no image hash, length 0.
fn captured_sticker(is_favorite: bool) -> StickerAction {
    let mut action = StickerAction::default();
    action.file_enc_sha256 = Some(vec![0xe1; 32]);
    action.media_key = Some(vec![0x6b; 32]);
    action.mimetype = Some("image/webp".to_string());
    action.height = Some(64);
    action.width = Some(64);
    action.direct_path = Some(DIRECT_PATH.to_string());
    action.file_length = Some(0);
    action.is_favorite = Some(is_favorite);
    action.device_id_hint = Some(0);
    action.is_lottie = Some(false);
    action.is_avatar_sticker = Some(false);
    action
}

fn favorite_event(action: StickerAction, ms: i64) -> Event {
    favorite_event_with_hash(action, ms, FAVORITE_HASH)
}

fn favorite_event_with_hash(action: StickerAction, ms: i64, filehash: &str) -> Event {
    Event::FavoriteStickerUpdate(
        FavoriteStickerUpdate::builder()
            .filehash(filehash.to_string())
            .timestamp(at_ms!(ms))
            .action(Box::new(action))
            .from_full_sync(false)
            .build(),
    )
}

#[test]
fn favorite_sticker_relays_a_downloadable_descriptor() {
    let expected = pb::StickerUpdate {
        filehash: FAVORITE_HASH.to_string(),
        timestamp: 1_791_470_722_582,
        from_full_sync: false,
        action: Some(Action::Favorite(pb::FavoriteStickerChange {
            is_favorite: Some(true),
            media: Some(pb::MediaDescriptor {
                direct_path: DIRECT_PATH.to_string(),
                media_key: vec![0x6b; 32],
                file_enc_sha256: vec![0xe1; 32],
                file_sha256: FAVORITE_SHA256.to_vec(),
                file_length: 0,
                mime_type: "image/webp".to_string(),
                media_type: pb::MediaType::Sticker as i32,
            }),
            url: None,
            width: Some(64),
            height: Some(64),
            is_lottie: Some(false),
            is_avatar_sticker: Some(false),
            image_hash: None,
            device_id_hint: Some(0),
        })),
    };
    let event = favorite_event(captured_sticker(true), 1_791_470_722_582);
    assert_eq!(sticker_of(event), expected);
}

/// Unfavoriting is the same action with the flag false, media included.
#[test]
fn unfavorite_sticker_keeps_is_favorite_false() {
    let update = sticker_of(favorite_event(captured_sticker(false), 1_791_470_728_676));
    let Some(Action::Favorite(change)) = update.action else {
        panic!("expected a favorite change, got {:?}", update.action);
    };
    assert_eq!(change.is_favorite, Some(false));
    assert!(
        change.media.is_some(),
        "the unfavorite still names the sticker"
    );
}

/// Without a path there is nothing to download: unset, never an empty one.
#[test]
fn favorite_sticker_without_a_direct_path_leaves_media_unset() {
    let mut action = captured_sticker(true);
    action.direct_path = None;
    let update = sticker_of(favorite_event(action, 1_791_470_722_582));
    let Some(Action::Favorite(change)) = update.action else {
        panic!("expected a favorite change, got {:?}", update.action);
    };
    assert_eq!(change.media, None);
}

/// A filehash that is not a base64 SHA-256 still crosses verbatim; the
/// descriptor goes out without a hash rather than with a guessed one.
#[test]
fn favorite_sticker_with_a_filehash_that_is_not_a_sha256_leaves_file_sha256_empty() {
    for filehash in ["not base64 at all", "c2hvcnQ="] {
        let event = favorite_event_with_hash(captured_sticker(true), 1, filehash);
        let update = sticker_of(event);
        assert_eq!(update.filehash, filehash);
        let Some(Action::Favorite(change)) = update.action else {
            panic!("expected a favorite change, got {:?}", update.action);
        };
        let media = change.media.expect("the path is there");
        assert_eq!(media.file_sha256, Vec::<u8>::new(), "for {filehash:?}");
    }
}

fn remove_recent_event(last_sent: Option<i64>) -> Event {
    let mut action = RemoveRecentStickerAction::default();
    action.last_sticker_sent_ts = last_sent;
    Event::RemoveRecentStickerUpdate(
        RemoveRecentStickerUpdate::builder()
            .filehash(RECENT_HASH.to_string())
            .timestamp(at_ms!(1_791_470_737_731))
            .action(Box::new(action))
            .from_full_sync(false)
            .build(),
    )
}

/// The capture: 1791391203204 is already ms (the day before), so it crosses
/// as sent.
#[test]
fn remove_recent_sticker_relays_the_last_sent_time_in_ms_verbatim() {
    let expected = pb::StickerUpdate {
        filehash: RECENT_HASH.to_string(),
        timestamp: 1_791_470_737_731,
        from_full_sync: false,
        action: Some(Action::RemoveRecent(pb::RemoveRecentStickerChange {
            last_sticker_sent_ts: Some(1_791_391_203_204),
        })),
    };
    assert_eq!(
        sticker_of(remove_recent_event(Some(1_791_391_203_204))),
        expected
    );
}

#[test]
fn remove_recent_sticker_without_a_time_leaves_it_unset() {
    let update = sticker_of(remove_recent_event(None));
    assert_eq!(
        update.action,
        Some(Action::RemoveRecent(pb::RemoveRecentStickerChange {
            last_sticker_sent_ts: None
        }))
    );
}
