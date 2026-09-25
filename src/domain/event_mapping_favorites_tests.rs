//! Issue #48 (upstream #1544): `Event::FavoritesUpdate` maps to the typed
//! `FavoritesChanged`, not the Raw catch-all. Its own file because
//! `event_mapping_tests.rs` is already past the 500-line rule.

use super::*;
use wa::sync_action_value::FavoritesAction;
use wa::sync_action_value::favorites_action::Favorite;
use wacore::types::events::FavoritesUpdate;

use crate::proto::v1::event_envelope::Event as PbEvent;

const GROUP_JID: &str = "120363041234567890@g.us";
const LID_JID: &str = "169815004184633@lid";
const MUTATION_MS: i64 = 1_758_800_000_123;

fn favorites_event(ids: &[Option<&str>]) -> Event {
    let action = FavoritesAction {
        favorites: ids
            .iter()
            .map(|id| Favorite {
                id: id.map(str::to_string),
            })
            .collect(),
    };
    Event::FavoritesUpdate(
        FavoritesUpdate::builder()
            .timestamp(wacore::time::from_millis_or_now(MUTATION_MS))
            .action(Box::new(action))
            .from_full_sync(true)
            .build(),
    )
}

fn mapped_favorites(event: &Event) -> pb::FavoritesChanged {
    match map_event(event).into_iter().next() {
        Some(PbEvent::FavoritesChanged(f)) => f,
        other => panic!("expected favorites changed, got {other:?}"),
    }
}

// The ids go out in the phone's order and as the phone spelled them: a LID
// stays a LID, nothing is rewritten.
#[test]
fn maps_favorites_list_in_order_and_verbatim() {
    let f = mapped_favorites(&favorites_event(&[Some(LID_JID), Some(GROUP_JID)]));
    assert_eq!(f.chats, vec![LID_JID.to_string(), GROUP_JID.to_string()]);
    assert_eq!(f.timestamp, MUTATION_MS);
    assert!(f.from_full_sync);
}

// An empty list is the "no favorites" state: it must still reach the consumer,
// or the last favorite could never be removed.
#[test]
fn maps_empty_favorites_to_an_event_with_no_chats() {
    let f = mapped_favorites(&favorites_event(&[]));
    assert!(f.chats.is_empty());
    assert_eq!(f.timestamp, MUTATION_MS);
}

// `id` is optional on the wire. A missing one is skipped, never relayed as an
// empty string the consumer would read as a chat.
#[test]
fn skips_a_favorite_without_an_id() {
    let f = mapped_favorites(&favorites_event(&[None, Some(GROUP_JID)]));
    assert_eq!(f.chats, vec![GROUP_JID.to_string()]);
}

// `raw` is the action protobuf itself, entries without an id included.
#[test]
fn favorites_raw_decodes_back_to_the_whole_action() {
    let f = mapped_favorites(&favorites_event(&[None, Some(GROUP_JID)]));
    let action = FavoritesAction::decode_from_slice(&f.raw).unwrap();
    assert_eq!(action.favorites.len(), 2);
    assert_eq!(action.favorites[1].id.as_deref(), Some(GROUP_JID));
}
