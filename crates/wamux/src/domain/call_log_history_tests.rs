//! #215: a call-log record from a history sync, routed through `map_event`, so
//! each test also proves the event does not fall into the RawEvent catch-all.
//! The record reuses the shape of #153's captured answered call (seconds for
//! `start_time` and `duration`); the history-only fields are hand set. History
//! call logs arrive only at pairing, so none was captured live.

#![allow(clippy::field_reassign_with_default)] // the generated records are built by assignment

use std::str::FromStr;

use wacore::types::events::{CallLogHistory, Event};
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
const GROUP: &str = "120363000000000215@g.us";
const CALL_ID: &str = "009F90B557B94899EC85D0D3E67B6215";

fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

fn history_of(event: Event) -> pb::CallLogHistory {
    match map_event(&event).as_slice() {
        [PbEvent::CallLogHistory(history)] => history.clone(),
        other => panic!("expected one CallLogHistory, got {other:?}"),
    }
}

/// A group voice call as the phone recorded it, `is_incoming` included.
fn full_record() -> CallLogRecord {
    let mut callee = ParticipantInfo::default();
    callee.user_jid = Some(CALLEE.to_string());
    callee.call_result = Some(CallResult::MISSED);
    let mut record = CallLogRecord::default();
    record.call_result = Some(CallResult::CONNECTED);
    record.is_dnd_mode = Some(true);
    record.silence_reason = Some(SilenceReason::PRIVACY);
    record.duration = Some(31);
    record.start_time = Some(1_791_489_445);
    record.is_incoming = Some(true);
    record.is_video = Some(true);
    record.is_call_link = Some(true);
    record.call_link_token = Some("link-token".to_string());
    record.scheduled_call_id = Some("scheduled-1".to_string());
    record.call_id = Some(CALL_ID.to_string());
    record.call_creator_jid = Some(CALLER.to_string());
    record.group_jid = Some(GROUP.to_string());
    record.participants = vec![callee];
    record.call_type = Some(CallType::VOICE_CHAT);
    record
}

fn full_event(from_me: Option<bool>) -> Event {
    Event::CallLogHistory(
        CallLogHistory::builder()
            .call_creator_jid(Jid::from_str(CALLER).expect("test jid"))
            .maybe_from_me(from_me)
            .timestamp(wacore::time::from_secs(1_791_489_445).expect("test instant"))
            .record(Box::new(full_record()))
            .sync_type(4)
            .chunk_order(2)
            .build(),
    )
}

#[test]
fn a_full_record_crosses_with_the_record_fields_as_call_log_update_maps_them() {
    let expected = pb::CallLogHistory {
        call_id: Some(CALL_ID.to_string()),
        call_creator: wire(CALLER),
        from_me: Some(true),
        sync_type: Some(4),
        chunk_order: Some(2),
        result: pb::CallLogResult::Connected as i32,
        is_dnd_mode: Some(true),
        silence_reason: pb::CallLogSilenceReason::Privacy as i32,
        duration_seconds: Some(31),
        start_time: Some(1_791_489_445_000),
        is_video: Some(true),
        is_call_link: Some(true),
        call_link_token: Some("link-token".to_string()),
        scheduled_call_id: Some("scheduled-1".to_string()),
        group: wire(GROUP),
        participants: vec![pb::CallLogParticipant {
            user: wire(CALLEE),
            result: pb::CallLogResult::Missed as i32,
        }],
        call_type: pb::CallLogType::VoiceChat as i32,
    };
    assert_eq!(history_of(full_event(Some(true))), expected);
}

#[test]
fn an_unknown_direction_is_unset_not_false() {
    assert_eq!(history_of(full_event(None)).from_me, None);
    assert_eq!(history_of(full_event(Some(false))).from_me, Some(false));
}

/// An empty record with nothing the library could derive: every optional stays
/// unset, closed enums are UNSPECIFIED, and the record's `is_incoming` is never
/// turned into a direction.
#[test]
fn an_empty_record_crosses_as_unset() {
    let mut record = CallLogRecord::default();
    record.is_incoming = Some(true);
    let event = Event::CallLogHistory(CallLogHistory::builder().record(Box::new(record)).build());
    assert_eq!(history_of(event), pb::CallLogHistory::default());
}
