//! #153 (part 2b of #141): a saved contact deleted on a linked device, routed
//! through `map_event`, so each test also proves the event no longer falls
//! into the RawEvent catch-all. The removal is the one the user made on the
//! pessoal phone on 2026-10-08 at 19:59:05Z (read off the production
//! subscription; jid anonymized). It carried no action timestamp.

use std::str::FromStr;

use wacore::types::events::{ContactRemoved, ContactUpdate, Event};
use whatsapp_rust::Jid;

use crate::domain::event_mapping::map_event;
use crate::proto::v1 as pb;
use crate::proto::v1::event_envelope::Event as PbEvent;

const CONTACT: &str = "5511900000003@s.whatsapp.net";
const REMOVED_MS: i64 = 1_791_489_545_312;

/// The instant `ms` after the epoch, as the library's builders take it. A
/// macro because wamux does not depend on chrono and cannot name the type.
macro_rules! at_ms {
    ($ms:expr) => {
        wacore::time::from_millis($ms).expect("test instant")
    };
}

/// The one ContactUpdate the event maps to; anything else is a failure.
fn contact_of(event: Event) -> pb::ContactUpdate {
    match map_event(&event).as_slice() {
        [PbEvent::Contact(update)] => update.clone(),
        other => panic!("expected one ContactUpdate, got {other:?}"),
    }
}

fn lib(value: &str) -> Jid {
    Jid::from_str(value).expect("test jid")
}

#[test]
fn contact_removed_relays_the_captured_removal() {
    let event = Event::ContactRemoved(
        ContactRemoved::builder()
            .jid(lib(CONTACT))
            .timestamp(at_ms!(REMOVED_MS))
            .from_full_sync(false)
            .build(),
    );
    let expected = pb::ContactUpdate {
        jid: Some(pb::Jid {
            value: CONTACT.to_string(),
        }),
        timestamp: REMOVED_MS,
        action_timestamp: None,
        from_full_sync: false,
        action: None,
        removed: true,
    };
    assert_eq!(contact_of(event), expected);
}

#[test]
fn contact_removed_keeps_the_action_timestamp() {
    let event = Event::ContactRemoved(
        ContactRemoved::builder()
            .jid(lib(CONTACT))
            .timestamp(at_ms!(REMOVED_MS))
            .action_timestamp(at_ms!(REMOVED_MS - 5_000))
            .from_full_sync(true)
            .build(),
    );
    let update = contact_of(event);
    assert_eq!(
        (
            update.action_timestamp,
            update.from_full_sync,
            update.removed
        ),
        (Some(REMOVED_MS - 5_000), true, true)
    );
}

/// The edit branch of the same mutation stays an edit: `removed` false and
/// the action set.
#[test]
fn contact_update_is_not_removed() {
    let event = Event::ContactUpdate(
        ContactUpdate::builder()
            .jid(lib(CONTACT))
            .timestamp(at_ms!(REMOVED_MS))
            .action(Box::default())
            .from_full_sync(false)
            .build(),
    );
    let update = contact_of(event);
    assert!(!update.removed);
    assert_eq!(update.action, Some(pb::ContactAction::default()));
}
