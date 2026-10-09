//! A call that must not ring (#151, part 4 of #141): today an offer replayed
//! from the offline queue. It reached the socket as RawEvent with the
//! library's JSON; it crosses as the `missed` action of a CallEvent.

use wacore::types::call::{MissedCall, MissedReason};
use wamux_types::relay_lib_jid;

use crate::proto::v1 as pb;

pub fn missed_call_event_of(missed: &MissedCall) -> pb::CallEvent {
    pb::CallEvent {
        from: relay_lib_jid(&missed.from),
        call_id: missed.call_id.clone(),
        timestamp: missed.timestamp.timestamp_millis(),
        // The CallEvent's own flag says "replayed from the offline queue", which
        // is exactly the Offline reason (#151).
        offline: matches!(missed.reason, MissedReason::Offline),
        action: Some(pb::call_event::Action::Missed(pb::CallMissed {
            reason: missed_reason_of(&missed.reason) as i32,
        })),
        ..pb::CallEvent::default()
    }
}

fn missed_reason_of(reason: &MissedReason) -> pb::MissedCallReason {
    match reason {
        MissedReason::Offline => pb::MissedCallReason::Offline,
        MissedReason::Remote => pb::MissedCallReason::Remote,
        // `MissedReason` is non_exhaustive: the compiler demands this arm, and a
        // reason the library adds later must cross as UNKNOWN, never UNSPECIFIED.
        _ => pb::MissedCallReason::Unknown,
    }
}

#[cfg(test)]
#[path = "missed_call_tests.rs"]
mod tests;
