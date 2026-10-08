//! #153 (part 2b of #141): the call log, routed through `map_event`, so each
//! test also proves the event no longer falls into the RawEvent catch-all.
//! The two captured calls are the ones the user placed from pessoal to trabalho
//! on 2026-10-08 at 19:57Z (answered, 30-31 s on the phones) and 19:59Z
//! (rejected), read off the production subscription; jids anonymized. They
//! measured `start_time` and `duration` in seconds. The group call is
//! hand-built: none was captured.

#![allow(clippy::field_reassign_with_default)] // the generated records are built by assignment

use std::str::FromStr;

use wacore::types::events::{CallLogSync, Event};
use whatsapp_rust::Jid;
use whatsapp_rust::waproto::whatsapp::CallLogRecord;
use whatsapp_rust::waproto::whatsapp::call_log_record::{
    CallResult, CallType, ParticipantInfo, SilenceReason,
};

use crate::domain::event_mapping::map_event;
use crate::proto::v1 as pb;
use crate::proto::v1::event_envelope::Event as PbEvent;

const CALLER: &str = "100000000000001@lid";
const CALLEE: &str = "100000000000002@lid";
const ANSWERED_ID: &str = "009F90B557B94899EC85D0D3E67B63E8";
const REJECTED_ID: &str = "00DE7D17830FAF8D87B0F4A86C956BF0";

/// The instant `ms` after the epoch, as the library's builders take it. A
/// macro because wamux does not depend on chrono and cannot name the type.
macro_rules! at_ms {
    ($ms:expr) => {
        wacore::time::from_millis($ms).expect("test instant")
    };
}

fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

/// The one CallLogUpdate the event maps to; anything else is a failure.
fn call_log_of(event: Event) -> pb::CallLogUpdate {
    match map_event(&event).as_slice() {
        [PbEvent::CallLog(update)] => update.clone(),
        other => panic!("expected one CallLogUpdate, got {other:?}"),
    }
}

/// A call log mutation written by this account's phone, created by `CALLER`.
fn sync_event(call_id: &str, ms: i64, record: CallLogRecord) -> Event {
    Event::CallLogSync(
        CallLogSync::builder()
            .call_creator_jid(Jid::from_str(CALLER).expect("test jid"))
            .call_id(call_id.to_string())
            .from_me(true)
            .timestamp(at_ms!(ms))
            .record(Box::new(record))
            .from_full_sync(false)
            .build(),
    )
}

fn participant(user_jid: Option<&str>, result: CallResult) -> ParticipantInfo {
    let mut info = ParticipantInfo::default();
    info.user_jid = user_jid.map(str::to_string);
    info.call_result = Some(result);
    info
}

/// The record of a one-to-one voice call as the phone wrote it, including the
/// fields the contract leaves out (`is_incoming`, the copies of the index).
fn captured_record(call_id: &str, result: CallResult, duration: i64, start: i64) -> CallLogRecord {
    let mut record = CallLogRecord::default();
    record.call_result = Some(result);
    record.is_dnd_mode = Some(false);
    record.silence_reason = Some(SilenceReason::NONE);
    record.duration = Some(duration);
    record.start_time = Some(start);
    record.is_incoming = Some(false);
    record.is_video = Some(false);
    record.call_id = Some(call_id.to_string());
    record.call_creator_jid = Some(CALLER.to_string());
    record.participants = vec![participant(Some(CALLEE), result)];
    record.call_type = Some(CallType::REGULAR);
    record
}

/// The contract's message for a captured one-to-one voice call.
fn captured_update(
    call_id: &str,
    result: pb::CallLogResult,
    times: (i64, i64, i64),
) -> pb::CallLogUpdate {
    let (mutation_ms, duration_seconds, start_ms) = times;
    pb::CallLogUpdate {
        call_creator: wire(CALLER),
        call_id: call_id.to_string(),
        from_me: true,
        timestamp: mutation_ms,
        from_full_sync: false,
        result: result as i32,
        is_dnd_mode: Some(false),
        silence_reason: pb::CallLogSilenceReason::None as i32,
        duration_seconds: Some(duration_seconds),
        start_time: Some(start_ms),
        is_video: Some(false),
        is_call_link: None,
        call_link_token: None,
        scheduled_call_id: None,
        group: None,
        participants: vec![pb::CallLogParticipant {
            user: wire(CALLEE),
            result: result as i32,
        }],
        call_type: pb::CallLogType::Regular as i32,
    }
}

/// The answered call: `start_time` was 1791489445 s, the offer reached the
/// callee at 1791489445657 ms, so it crosses as ms; `duration` was 31 with
/// the phones showing 30 and 31 s, so it crosses as seconds, verbatim.
#[test]
fn call_log_relays_the_captured_connected_call() {
    let record = captured_record(ANSWERED_ID, CallResult::CONNECTED, 31, 1_791_489_445);
    let expected = captured_update(
        ANSWERED_ID,
        pb::CallLogResult::Connected,
        (1_791_489_479_672, 31, 1_791_489_445_000),
    );
    assert_eq!(
        call_log_of(sync_event(ANSWERED_ID, 1_791_489_479_672, record)),
        expected
    );
}

#[test]
fn call_log_relays_the_captured_rejected_call() {
    let record = captured_record(REJECTED_ID, CallResult::REJECTED, 0, 1_791_489_560);
    let expected = captured_update(
        REJECTED_ID,
        pb::CallLogResult::Rejected,
        (1_791_489_563_369, 0, 1_791_489_560_000),
    );
    assert_eq!(
        call_log_of(sync_event(REJECTED_ID, 1_791_489_563_369, record)),
        expected
    );
}

/// An empty record: closed enums absent cross as UNSPECIFIED (never UNKNOWN),
/// optionals stay unset, and an empty group jid is no jid.
#[test]
fn call_log_absent_fields_cross_as_unset() {
    let mut record = CallLogRecord::default();
    record.group_jid = Some(String::new());
    let expected = pb::CallLogUpdate {
        call_creator: wire(CALLER),
        call_id: ANSWERED_ID.to_string(),
        from_me: true,
        timestamp: 1_791_489_479_672,
        ..pb::CallLogUpdate::default()
    };
    let update = call_log_of(sync_event(ANSWERED_ID, 1_791_489_479_672, record));
    assert_eq!(update, expected);
    assert_eq!(update.result, pb::CallLogResult::Unspecified as i32);
    assert_eq!(update.call_type, pb::CallLogType::Unspecified as i32);
}

fn result_of(result: CallResult) -> (i32, i32) {
    let mut record = CallLogRecord::default();
    record.call_result = Some(result);
    record.participants = vec![participant(Some(CALLEE), result)];
    let update = call_log_of(sync_event(ANSWERED_ID, 0, record));
    (update.result, update.participants[0].result)
}

fn silence_of(reason: SilenceReason) -> i32 {
    let mut record = CallLogRecord::default();
    record.silence_reason = Some(reason);
    call_log_of(sync_event(ANSWERED_ID, 0, record)).silence_reason
}

fn type_of(call_type: CallType) -> i32 {
    let mut record = CallLogRecord::default();
    record.call_type = Some(call_type);
    call_log_of(sync_event(ANSWERED_ID, 0, record)).call_type
}

/// Every value of the three library enums, each to its own case, for the call
/// and for a participant alike.
#[test]
fn call_log_every_library_enum_value_maps_to_its_case() {
    use pb::CallLogResult as R;
    let results = [
        (CallResult::CONNECTED, R::Connected),
        (CallResult::REJECTED, R::Rejected),
        (CallResult::CANCELLED, R::Cancelled),
        (CallResult::ACCEPTEDELSEWHERE, R::AcceptedElsewhere),
        (CallResult::MISSED, R::Missed),
        (CallResult::INVALID, R::Invalid),
        (CallResult::UNAVAILABLE, R::Unavailable),
        (CallResult::UPCOMING, R::Upcoming),
        (CallResult::FAILED, R::Failed),
        (CallResult::ABANDONED, R::Abandoned),
        (CallResult::ONGOING, R::Ongoing),
    ];
    for (library, wire) in results {
        assert_eq!(
            result_of(library),
            (wire as i32, wire as i32),
            "{library:?}"
        );
    }
    use pb::CallLogSilenceReason as S;
    let reasons = [
        (SilenceReason::NONE, S::None),
        (SilenceReason::SCHEDULED, S::Scheduled),
        (SilenceReason::PRIVACY, S::Privacy),
        (SilenceReason::LIGHTWEIGHT, S::Lightweight),
    ];
    for (library, wire) in reasons {
        assert_eq!(silence_of(library), wire as i32, "{library:?}");
    }
    use pb::CallLogType as T;
    let types = [
        (CallType::REGULAR, T::Regular),
        (CallType::SCHEDULED_CALL, T::ScheduledCall),
        (CallType::VOICE_CHAT, T::VoiceChat),
    ];
    for (library, wire) in types {
        assert_eq!(type_of(library), wire as i32, "{library:?}");
    }
}

/// A group voice chat with a call link: the group crosses as a jid, the link
/// fields verbatim, and a participant with no jid (absent or empty) is
/// skipped rather than crossing as an empty one.
#[test]
fn call_log_group_call_carries_group_and_skips_empty_participants() {
    let mut record = CallLogRecord::default();
    record.group_jid = Some("120363000000000153@g.us".to_string());
    record.is_video = Some(true);
    record.is_call_link = Some(true);
    record.call_link_token = Some("link-token".to_string());
    record.scheduled_call_id = Some("scheduled-1".to_string());
    record.call_type = Some(CallType::VOICE_CHAT);
    record.participants = vec![
        participant(Some(""), CallResult::MISSED),
        participant(Some(CALLEE), CallResult::MISSED),
        participant(None, CallResult::MISSED),
    ];
    let update = call_log_of(sync_event(ANSWERED_ID, 0, record));
    assert_eq!(update.group, wire("120363000000000153@g.us"));
    assert_eq!(
        (update.is_video, update.is_call_link),
        (Some(true), Some(true))
    );
    assert_eq!(update.call_link_token.as_deref(), Some("link-token"));
    assert_eq!(update.scheduled_call_id.as_deref(), Some("scheduled-1"));
    assert_eq!(update.call_type, pb::CallLogType::VoiceChat as i32);
    assert_eq!(
        update.participants,
        vec![pb::CallLogParticipant {
            user: wire(CALLEE),
            result: pb::CallLogResult::Missed as i32,
        }]
    );
}

/// Direction is `from_me` alone: the library documents `is_incoming` as
/// inverted in mutations WA Web wrote, so flipping it changes nothing.
#[test]
fn call_log_ignores_is_incoming() {
    let mut incoming = captured_record(ANSWERED_ID, CallResult::CONNECTED, 31, 1_791_489_445);
    incoming.is_incoming = Some(true);
    let outgoing = captured_record(ANSWERED_ID, CallResult::CONNECTED, 31, 1_791_489_445);
    assert_eq!(
        call_log_of(sync_event(ANSWERED_ID, 1_791_489_479_672, incoming)),
        call_log_of(sync_event(ANSWERED_ID, 1_791_489_479_672, outgoing))
    );
}

/// A nonsense start time saturates instead of overflowing (wire_time rule).
#[test]
fn call_log_start_time_saturates() {
    let mut record = CallLogRecord::default();
    record.start_time = Some(i64::MAX);
    let update = call_log_of(sync_event(ANSWERED_ID, 0, record));
    assert_eq!(update.start_time, Some(i64::MAX));
}
