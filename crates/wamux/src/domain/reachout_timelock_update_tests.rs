//! #215: the reachout timelock, routed through `map_event`, so each test also
//! proves the event does not fall into the RawEvent catch-all. Hand built from
//! the library's type: the server sends it on its own schedule.

#![allow(clippy::field_reassign_with_default)] // the state is built by assignment

use std::str::FromStr;

use wacore::types::events::{Event, ReachoutTimelock, ReachoutTimelockUpdate};
use whatsapp_rust::Jid;

use crate::domain::event_mapping::map_event;
use crate::proto::v1 as pb;
use crate::proto::v1::event_envelope::Event as PbEvent;

const SERVER: &str = "s.whatsapp.net";

fn timelock_of(event: Event) -> pb::ReachoutTimelockUpdate {
    match map_event(&event).as_slice() {
        [PbEvent::ReachoutTimelock(update)] => update.clone(),
        other => panic!("expected one ReachoutTimelockUpdate, got {other:?}"),
    }
}

fn state(
    enforcement_type: Option<&str>,
    is_active: Option<bool>,
    ends: Option<&str>,
) -> ReachoutTimelock {
    let mut state = ReachoutTimelock::default();
    state.enforcement_type = enforcement_type.map(str::to_string);
    state.is_active = is_active;
    state.time_enforcement_ends = ends.map(str::to_string);
    state
}

#[test]
fn every_field_crosses_as_the_server_sent_it() {
    let event = Event::ReachoutTimelockUpdate(
        ReachoutTimelockUpdate::builder()
            .state(state(
                Some("SOME_ENFORCEMENT"),
                Some(true),
                Some("1792000000"),
            ))
            .from(Jid::from_str(SERVER).expect("server jid"))
            .stanza_id("3EB0TIMELOCK".to_string())
            .offline("1".to_string())
            .build(),
    );
    let update = timelock_of(event);
    assert_eq!(update.enforcement_type.as_deref(), Some("SOME_ENFORCEMENT"));
    assert_eq!(update.is_active, Some(true));
    assert_eq!(
        update.time_enforcement_ends.as_deref(),
        Some("1792000000"),
        "the deadline is the server's string, not reinterpreted"
    );
    assert_eq!(
        update.from,
        Some(pb::Jid {
            value: SERVER.to_string()
        })
    );
    assert_eq!(update.stanza_id.as_deref(), Some("3EB0TIMELOCK"));
    assert_eq!(update.offline.as_deref(), Some("1"));
}

#[test]
fn an_absent_field_stays_unset() {
    let event = Event::ReachoutTimelockUpdate(
        ReachoutTimelockUpdate::builder()
            .state(state(None, None, None))
            .build(),
    );
    let update = timelock_of(event);
    assert_eq!(update.enforcement_type, None);
    assert_eq!(
        update.is_active, None,
        "absent makes no claim, it is not false"
    );
    assert_eq!(update.time_enforcement_ends, None);
    assert_eq!(update.from, None);
    assert_eq!(update.stanza_id, None);
    assert_eq!(update.offline, None);
}

#[test]
fn a_lifted_restriction_is_false_not_unset() {
    let event = Event::ReachoutTimelockUpdate(
        ReachoutTimelockUpdate::builder()
            .state(state(None, Some(false), None))
            .build(),
    );
    assert_eq!(timelock_of(event).is_active, Some(false));
}
