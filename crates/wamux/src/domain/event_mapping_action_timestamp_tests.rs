//! Upstream #1562 (#86): each chat-action update carries `action_timestamp`,
//! the instant the mutation itself held, next to `timestamp`, which falls back
//! to the epoch or the dispatch time when the mutation held none. Both
//! app-state updates (#134) and contact updates (#133) carry it as a typed
//! field. These tests pin that the edge gets it, both ways.

use super::*;
use wacore::types::events::{ArchiveUpdate, ContactUpdate};

use crate::proto::v1::event_envelope::Event as PbEvent;

const CHAT_JID: &str = "120363041234567890@g.us";
const MUTATION_SECS: i64 = 1_717_932_000;

fn app_state_of(event: &Event) -> pb::AppStateUpdate {
    match map_event(event).into_iter().next() {
        Some(PbEvent::AppState(s)) => s,
        other => panic!("expected an app-state update, got {other:?}"),
    }
}

fn archive(action_timestamp: Option<i64>) -> Event {
    let at = |secs| wacore::time::from_secs(secs).unwrap();
    Event::ArchiveUpdate(
        ArchiveUpdate::builder()
            .jid(CHAT_JID.parse().unwrap())
            .timestamp(at(action_timestamp.unwrap_or(0)))
            .maybe_action_timestamp(action_timestamp.map(at))
            .action(Box::default())
            .from_full_sync(false)
            .build(),
    )
}

#[test]
fn app_state_carries_the_mutations_own_timestamp() {
    let update = app_state_of(&archive(Some(MUTATION_SECS)));
    assert_eq!(update.action_timestamp, Some(MUTATION_SECS * 1000));
    assert_eq!(update.timestamp, MUTATION_SECS * 1000);
}

#[test]
fn app_state_without_a_mutation_timestamp_leaves_it_unset() {
    // The case #1562 exists for: `timestamp` is the epoch fallback, and only
    // an unset `action_timestamp` tells the edge it is not real.
    let update = app_state_of(&archive(None));
    assert_eq!(update.action_timestamp, None);
    assert_eq!(update.timestamp, 0);
}

#[test]
fn contact_update_carries_the_mutations_own_timestamp() {
    let at = wacore::time::from_secs(MUTATION_SECS).unwrap();
    let mapped = map_event(&Event::ContactUpdate(
        ContactUpdate::builder()
            .jid("559980000001@s.whatsapp.net".parse().unwrap())
            .timestamp(at)
            .action_timestamp(at)
            .action(Box::default())
            .from_full_sync(false)
            .build(),
    ));
    match mapped.into_iter().next() {
        Some(PbEvent::Contact(c)) => {
            assert_eq!(c.action_timestamp, Some(MUTATION_SECS * 1000));
            assert_eq!(c.timestamp, MUTATION_SECS * 1000);
        }
        other => panic!("expected a contact update, got {other:?}"),
    }
}
