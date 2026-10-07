//! Upstream #1562 (#86): each chat-action update carries `action_timestamp`,
//! the instant the mutation itself held, next to `timestamp`, which falls back
//! to the epoch or the dispatch time when the mutation held none. App-state
//! updates still relay the library struct as JSON in `raw` (typed in #134);
//! a contact update carries it as a typed field since #133. These tests pin
//! that the edge gets it, both ways.

use super::*;
use wacore::types::events::{ArchiveUpdate, ContactUpdate};

use crate::proto::v1::event_envelope::Event as PbEvent;

const CHAT_JID: &str = "120363041234567890@g.us";
const MUTATION_SECS: i64 = 1_717_932_000;

fn raw_json_of(event: &Event) -> serde_json::Value {
    let raw = match map_event(event).into_iter().next() {
        Some(PbEvent::AppState(s)) => s.raw,
        other => panic!("expected an app-state update, got {other:?}"),
    };
    serde_json::from_slice(&raw).unwrap()
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
fn app_state_raw_carries_the_mutations_own_timestamp() {
    let json = raw_json_of(&archive(Some(MUTATION_SECS)));
    assert!(json["action_timestamp"].is_string(), "{json}");
    assert_eq!(json["action_timestamp"], json["timestamp"]);
}

#[test]
fn app_state_raw_says_null_when_the_mutation_had_no_timestamp() {
    // The case #1562 exists for: `timestamp` is the epoch fallback, and only
    // a present-but-null `action_timestamp` tells the edge it is not real.
    let json = raw_json_of(&archive(None));
    let fields = json.as_object().unwrap();
    assert!(fields.contains_key("action_timestamp"), "{json}");
    assert!(json["action_timestamp"].is_null());
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
