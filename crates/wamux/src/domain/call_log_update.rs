//! One entry of the primary device's call log: synced through app state
//! (#153), or carried by a history sync (#215). Both map the record the same way.

use wacore::types::events::{CallLogHistory, CallLogSync};
use wamux_types::{relay_jid, relay_lib_jid, relay_optional_lib_jid};
use whatsapp_rust::waproto::whatsapp::CallLogRecord;
use whatsapp_rust::waproto::whatsapp::call_log_record::{
    CallResult, CallType, ParticipantInfo, SilenceReason,
};

use crate::domain::wire_time::millis_from_signed_seconds;
use crate::proto::v1 as pb;

/// The index fields and the record, flattened. `record.is_incoming` and the
/// record's copies of the index (`call_id`, `call_creator_jid`) are left out
/// (see `CallLogUpdate` in call.proto).
pub fn call_log_update_of(sync: &CallLogSync) -> pb::CallLogUpdate {
    let fields: RecordFields = record_fields_of(&sync.record);
    pb::CallLogUpdate {
        call_creator: relay_lib_jid(&sync.call_creator_jid),
        call_id: sync.call_id.clone(),
        from_me: sync.from_me,
        timestamp: sync.timestamp.timestamp_millis(),
        from_full_sync: sync.from_full_sync,
        result: fields.result,
        is_dnd_mode: fields.is_dnd_mode,
        silence_reason: fields.silence_reason,
        duration_seconds: fields.duration_seconds,
        start_time: fields.start_time,
        is_video: fields.is_video,
        is_call_link: fields.is_call_link,
        call_link_token: fields.call_link_token,
        scheduled_call_id: fields.scheduled_call_id,
        group: fields.group,
        participants: fields.participants,
        call_type: fields.call_type,
    }
}

/// What a `CallLogRecord` says about the call, in contract terms. One mapping
/// for the app-state update and the history sync, so the two cannot drift.
struct RecordFields {
    result: i32,
    is_dnd_mode: Option<bool>,
    silence_reason: i32,
    duration_seconds: Option<i64>,
    start_time: Option<i64>,
    is_video: Option<bool>,
    is_call_link: Option<bool>,
    call_link_token: Option<String>,
    scheduled_call_id: Option<String>,
    group: Option<pb::Jid>,
    participants: Vec<pb::CallLogParticipant>,
    call_type: i32,
}

fn record_fields_of(record: &CallLogRecord) -> RecordFields {
    RecordFields {
        result: call_result_of(record.call_result) as i32,
        is_dnd_mode: record.is_dnd_mode,
        silence_reason: silence_reason_of(record.silence_reason) as i32,
        // Seconds verbatim: the name carries the unit (#153).
        duration_seconds: record.duration,
        // The server sends seconds; every instant in the contract is ms (#132).
        start_time: record.start_time.map(millis_from_signed_seconds),
        is_video: record.is_video,
        is_call_link: record.is_call_link,
        call_link_token: record.call_link_token.clone(),
        scheduled_call_id: record.scheduled_call_id.clone(),
        group: record.group_jid.clone().and_then(relay_jid),
        participants: record
            .participants
            .iter()
            .filter_map(participant_of)
            .collect(),
        call_type: call_type_of(record.call_type) as i32,
    }
}

/// A participant with no jid (absent or empty) is skipped, as in #149.
fn participant_of(info: &ParticipantInfo) -> Option<pb::CallLogParticipant> {
    let user = info.user_jid.clone().and_then(relay_jid)?;
    Some(pb::CallLogParticipant {
        user: Some(user),
        result: call_result_of(info.call_result) as i32,
    })
}

/// Absent is UNSPECIFIED. A variant this build does not name (the waproto enums
/// are `#[non_exhaustive]`) is UNKNOWN, never relayed as a wrong case (#149
/// rule); the library gives no number to carry alongside.
fn call_result_of(result: Option<CallResult>) -> pb::CallLogResult {
    use pb::CallLogResult as Kind;
    let Some(result) = result else {
        return Kind::Unspecified;
    };
    match result {
        CallResult::CONNECTED => Kind::Connected,
        CallResult::REJECTED => Kind::Rejected,
        CallResult::CANCELLED => Kind::Cancelled,
        CallResult::ACCEPTEDELSEWHERE => Kind::AcceptedElsewhere,
        CallResult::MISSED => Kind::Missed,
        CallResult::INVALID => Kind::Invalid,
        CallResult::UNAVAILABLE => Kind::Unavailable,
        CallResult::UPCOMING => Kind::Upcoming,
        CallResult::FAILED => Kind::Failed,
        CallResult::ABANDONED => Kind::Abandoned,
        CallResult::ONGOING => Kind::Ongoing,
        _ => Kind::Unknown,
    }
}

/// Same rules as `call_result_of`.
fn silence_reason_of(reason: Option<SilenceReason>) -> pb::CallLogSilenceReason {
    use pb::CallLogSilenceReason as Kind;
    let Some(reason) = reason else {
        return Kind::Unspecified;
    };
    match reason {
        SilenceReason::NONE => Kind::None,
        SilenceReason::SCHEDULED => Kind::Scheduled,
        SilenceReason::PRIVACY => Kind::Privacy,
        SilenceReason::LIGHTWEIGHT => Kind::Lightweight,
        _ => Kind::Unknown,
    }
}

/// Same rules as `call_result_of`.
fn call_type_of(call_type: Option<CallType>) -> pb::CallLogType {
    use pb::CallLogType as Kind;
    let Some(call_type) = call_type else {
        return Kind::Unspecified;
    };
    match call_type {
        CallType::REGULAR => Kind::Regular,
        CallType::SCHEDULED_CALL => Kind::ScheduledCall,
        CallType::VOICE_CHAT => Kind::VoiceChat,
        _ => Kind::Unknown,
    }
}

/// One call-log record from a history sync (#215): the record's fields as
/// `call_log_update_of` maps them, and the identity the library derived.
pub fn call_log_history_of(history: &CallLogHistory) -> pb::CallLogHistory {
    let fields: RecordFields = record_fields_of(&history.record);
    pb::CallLogHistory {
        call_id: history.record.call_id.clone(),
        call_creator: relay_optional_lib_jid(history.call_creator_jid.as_ref()),
        from_me: history.from_me,
        sync_type: history.sync_type,
        chunk_order: history.chunk_order,
        result: fields.result,
        is_dnd_mode: fields.is_dnd_mode,
        silence_reason: fields.silence_reason,
        duration_seconds: fields.duration_seconds,
        start_time: fields.start_time,
        is_video: fields.is_video,
        is_call_link: fields.is_call_link,
        call_link_token: fields.call_link_token,
        scheduled_call_id: fields.scheduled_call_id,
        group: fields.group,
        participants: fields.participants,
        call_type: fields.call_type,
    }
}

#[cfg(test)]
#[path = "call_log_update_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "call_log_history_tests.rs"]
mod history_tests;
