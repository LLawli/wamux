//! #151 (part 4 of #141): a call that must not ring, routed through
//! `map_event`, so each test also proves the event no longer falls into the
//! RawEvent catch-all. It crosses as a CallEvent whose action is `missed`.

use std::str::FromStr;

use wacore::types::call::{MissedCall, MissedReason};
use wacore::types::events::Event;
use whatsapp_rust::Jid;

use crate::domain::event_mapping::map_event;
use crate::proto::v1 as pb;
use crate::proto::v1::call_event::Action;
use crate::proto::v1::event_envelope::Event as PbEvent;

const CALLER: &str = "100000000000001@lid";
const CALL_ID: &str = "00AB12CD34EF5678";

/// The one CallEvent the event maps to; anything else is a failure.
fn call_of(event: Event) -> pb::CallEvent {
    match map_event(&event).as_slice() {
        [PbEvent::Call(call)] => call.clone(),
        other => panic!("expected one CallEvent, got {other:?}"),
    }
}

fn missed(reason: MissedReason, seconds: i64) -> Event {
    Event::MissedCall(MissedCall::new(
        Jid::from_str(CALLER).expect("test jid"),
        CALL_ID.to_string(),
        wacore::time::from_secs(seconds).expect("test instant"),
        reason,
    ))
}

/// An offer replayed from the offline queue: who, which call, when (ms), the
/// `offline` flag, and the reason as the action. Everything a `<call>` stanza
/// would add (stanza id, platform, routing) is absent here and stays unset.
#[test]
fn missed_offline_call_is_a_call_event() {
    let expected = pb::CallEvent {
        from: Some(pb::Jid {
            value: CALLER.to_string(),
        }),
        call_id: CALL_ID.to_string(),
        timestamp: 1_791_550_000_000,
        offline: true,
        action: Some(Action::Missed(pb::CallMissed {
            reason: pb::MissedCallReason::Offline as i32,
        })),
        ..pb::CallEvent::default()
    };
    assert_eq!(
        call_of(missed(MissedReason::Offline, 1_791_550_000)),
        expected
    );
}

/// A remote miss (needs `voip-control`, off in this build) is not an offline
/// replay.
#[test]
fn missed_remote_call_is_not_offline() {
    let call = call_of(missed(MissedReason::Remote, 1_791_550_000));
    assert!(!call.offline);
    assert_eq!(
        call.action,
        Some(Action::Missed(pb::CallMissed {
            reason: pb::MissedCallReason::Remote as i32,
        }))
    );
}

/// The stanza's seconds cross as ms, as every other time in the contract.
#[test]
fn missed_call_time_is_ms() {
    let call = call_of(missed(MissedReason::Offline, 1));
    assert_eq!(call.timestamp, 1_000);
}
