//! #135: the typed projection of a library `IncomingCall`, asserted as the
//! whole message, and the fixtures for the table of one hand-built value per
//! `CallAction` variant (`call_event_action_tests.rs`); the live captures are
//! in `call_event_capture_tests.rs`.

use std::str::FromStr;

use wacore::types::call::{CallAction, CallAudioCodec, IncomingCall, VideoState};
use wacore::types::group_call::{
    CallLinkMedia, GroupCallDevice, GroupCallEncRekey, GroupCallParticipant, GroupCallRelay,
    GroupCallRelayEndpoint, GroupCallUpdate, WaitingRoom, WaitingRoomUser,
};
use whatsapp_rust::Jid;

use super::*;
use crate::proto::v1::call_event::Action;

pub(super) const CALLER: &str = "100000000000001@lid";
pub(super) const CALLER_PN: &str = "5511900000001@s.whatsapp.net";
const MEMBER: &str = "100000000000002@lid";
const MEMBER_PN: &str = "5511900000002@s.whatsapp.net";
const MEMBER_DEVICE: &str = "100000000000002:3@lid";
pub(super) const GROUP: &str = "120363000000000135@g.us";
pub(super) const CALL_ID: &str = "CALL-135";
const STANZA_ID: &str = "STANZA-135";
const SECONDS: i64 = 1_791_457_942;

pub(super) fn lib(value: &str) -> Jid {
    Jid::from_str(value).expect("test jid parses")
}

pub(super) fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

/// The instant `seconds` after the epoch, as the library's builders take it.
/// A macro because wamux does not depend on chrono and cannot name the type.
macro_rules! at {
    ($seconds:expr) => {
        wacore::time::from_secs($seconds).expect("test instant")
    };
}

pub(super) fn codecs() -> Vec<CallAudioCodec> {
    vec![
        CallAudioCodec {
            enc: "opus".to_string(),
            rate: 16000,
        },
        CallAudioCodec {
            enc: "pcm".to_string(),
            rate: 8000,
        },
    ]
}

pub(super) fn wire_codecs() -> Vec<pb::CallAudioCodec> {
    vec![
        pb::CallAudioCodec {
            enc: "opus".to_string(),
            rate: 16000,
        },
        pb::CallAudioCodec {
            enc: "pcm".to_string(),
            rate: 8000,
        },
    ]
}

// ---------------------------------------------------------------------------
// Group-call structs, every public field set
// ---------------------------------------------------------------------------

fn device() -> GroupCallDevice {
    // `capability` is pub(crate): its public setter is `with_capability`.
    let mut device = GroupCallDevice::new(lib(MEMBER_DEVICE)).with_capability(7, vec![1, 2, 3]);
    device.platform = Some("iphone".to_string());
    device.pid = Some(4);
    device
}

fn wire_device() -> pb::GroupCallDevice {
    pb::GroupCallDevice {
        jid: wire(MEMBER_DEVICE),
        platform: Some("iphone".to_string()),
        pid: Some(4),
        capability_version: Some(7),
        capability: vec![1, 2, 3],
    }
}

fn relay() -> GroupCallRelay {
    let endpoint = GroupCallRelayEndpoint::builder()
        .relay_id(1)
        .token_id(2)
        .auth_token_id(3)
        .relay_name("gru1c02".to_string())
        .domain_name("edgeray-gru1-2.wt.whatsapp.com".to_string())
        .rtt_ms(24)
        .is_fna(true)
        .ipv4("192.0.2.10".to_string())
        .port(3478)
        .build();
    GroupCallRelay::builder()
        .transaction_id(9)
        .self_pid(5)
        .uuid("relay-uuid".to_string())
        .participant_uuid("participant-uuid".to_string())
        .attribute_padding(true)
        .warp_mi_tag_len(8)
        .endpoints(vec![endpoint])
        .build()
}

fn wire_relay() -> pb::GroupCallRelay {
    pb::GroupCallRelay {
        transaction_id: Some(9),
        self_pid: Some(5),
        uuid: "relay-uuid".to_string(),
        participant_uuid: "participant-uuid".to_string(),
        attribute_padding: true,
        warp_mi_tag_len: Some(8),
        endpoints: vec![pb::GroupCallRelayEndpoint {
            relay_id: 1,
            token_id: 2,
            auth_token_id: 3,
            relay_name: "gru1c02".to_string(),
            domain_name: Some("edgeray-gru1-2.wt.whatsapp.com".to_string()),
            rtt_ms: Some(24),
            is_fna: true,
            ipv4: Some("192.0.2.10".to_string()),
            port: Some(3478),
        }],
    }
}

pub(super) fn group_update() -> GroupCallUpdate {
    let participant = GroupCallParticipant::builder()
        .jid(lib(MEMBER))
        .pn(lib(MEMBER_PN))
        .state("connected".to_string())
        .participant_type("admin".to_string())
        .devices(vec![device()])
        .build();
    GroupCallUpdate::builder()
        .call_id(CALL_ID.to_string())
        .call_creator(lib(CALLER))
        .group_jid(lib(GROUP))
        .transaction_id(12)
        .media("video".to_string())
        .connected_limit(32)
        .joinable(true)
        .av_upgradable(true)
        .rekey_requested(true)
        .participants(vec![participant])
        .relay(relay())
        .build()
}

pub(super) fn wire_group_update() -> pb::GroupCallUpdate {
    pb::GroupCallUpdate {
        call_id: CALL_ID.to_string(),
        call_creator: wire(CALLER),
        group_jid: wire(GROUP),
        transaction_id: 12,
        media: "video".to_string(),
        connected_limit: 32,
        joinable: true,
        av_upgradable: true,
        rekey_requested: true,
        participants: vec![pb::GroupCallParticipant {
            jid: wire(MEMBER),
            // The library keeps it out of its JSON; the contract does not.
            pn: wire(MEMBER_PN),
            state: Some("connected".to_string()),
            participant_type: Some("admin".to_string()),
            devices: vec![wire_device()],
        }],
        relay: Some(wire_relay()),
    }
}

pub(super) fn enc_rekey() -> GroupCallEncRekey {
    GroupCallEncRekey::builder()
        .call_id(CALL_ID.to_string())
        .call_creator(lib(CALLER))
        .transaction_id(13)
        .key_generation(2)
        .encryption_type("msg".to_string())
        .encryption_version(3)
        .ciphertext(vec![9, 8, 7])
        .build()
}

pub(super) fn wire_enc_rekey() -> pb::GroupCallEncRekey {
    pb::GroupCallEncRekey {
        call_id: CALL_ID.to_string(),
        call_creator: wire(CALLER),
        transaction_id: 13,
        key_generation: 2,
        encryption_type: "msg".to_string(),
        encryption_version: 3,
        ciphertext: vec![9, 8, 7],
    }
}

pub(super) fn waiting_room() -> WaitingRoom {
    let with_pn = WaitingRoomUser::builder()
        .jid(lib(MEMBER))
        .pn(lib(MEMBER_PN))
        .state("waiting".to_string())
        .build();
    let without_pn = WaitingRoomUser::builder()
        .jid(lib(CALLER))
        .state("admitted".to_string())
        .build();
    WaitingRoom::builder()
        .call_id(CALL_ID.to_string())
        .call_creator(lib(CALLER))
        .link_token("link-token".to_string())
        .media(CallLinkMedia::Video)
        .enabled(true)
        .is_admin(true)
        .transaction_id(14)
        .users(vec![with_pn, without_pn])
        .build()
}

pub(super) fn wire_waiting_room() -> pb::CallWaitingRoom {
    pb::CallWaitingRoom {
        call_id: CALL_ID.to_string(),
        call_creator: wire(CALLER),
        link_token: "link-token".to_string(),
        media: pb::CallLinkMedia::Video as i32,
        enabled: true,
        is_admin: true,
        transaction_id: Some(14),
        users: vec![
            pb::WaitingRoomUser {
                jid: wire(MEMBER),
                pn: wire(MEMBER_PN),
                state: "waiting".to_string(),
            },
            pb::WaitingRoomUser {
                jid: wire(CALLER),
                pn: None,
                state: "admitted".to_string(),
            },
        ],
    }
}

/// The minimal call the library's test constructor builds around `action`,
/// spelled through the builder that constructor wraps.
pub(super) fn bare_call(action: CallAction) -> IncomingCall {
    IncomingCall::builder()
        .from(lib(CALLER))
        .stanza_id(STANZA_ID.to_string())
        .timestamp(at!(SECONDS))
        .offline(false)
        .action(action)
        .build()
}

/// What a bare call maps to around `action`: every optional unset.
pub(super) fn bare_wire(action: Action) -> pb::CallEvent {
    pb::CallEvent {
        from: wire(CALLER),
        call_id: CALL_ID.to_string(),
        call_creator: wire(CALLER),
        stanza_id: STANZA_ID.to_string(),
        timestamp: SECONDS * 1000,
        action: Some(action),
        ..Default::default()
    }
}

#[test]
fn a_video_state_the_library_does_not_name_keeps_its_code() {
    let call = bare_call(CallAction::VideoState {
        call_id: CALL_ID.to_string(),
        call_creator: lib(CALLER),
        state: VideoState::Unknown(42),
        orientation: None,
        dec: None,
    });
    let want = bare_wire(Action::VideoState(pb::CallVideoState {
        state: pb::CallVideoStateKind::Unknown as i32,
        state_code: 42,
        orientation: None,
        dec: None,
    }));
    assert_eq!(call_event_of(&call), want);
}

#[test]
fn incoming_call_fields_cross_by_value() {
    let call = IncomingCall::builder()
        .from(lib(CALLER))
        .stanza_id(STANZA_ID.to_string())
        .notify("Caller".to_string())
        .platform("smba".to_string())
        .version("2.26.39.72".to_string())
        .participant(lib(MEMBER_DEVICE))
        .recipient(lib(MEMBER))
        .timestamp(at!(SECONDS))
        .offline(true)
        .action(CallAction::Accept {
            call_id: CALL_ID.to_string(),
            call_creator: lib(CALLER),
            audio: codecs(),
        })
        .caller_username("caller.name".to_string())
        .video_orientation(2)
        .group(Box::new(group_update()))
        .build();
    let want = pb::CallEvent {
        from: wire(CALLER),
        call_id: CALL_ID.to_string(),
        call_creator: wire(CALLER),
        stanza_id: STANZA_ID.to_string(),
        notify: Some("Caller".to_string()),
        platform: Some("smba".to_string()),
        version: Some("2.26.39.72".to_string()),
        participant: wire(MEMBER_DEVICE),
        recipient: wire(MEMBER),
        // The stanza's `t` is unix seconds; the contract's instants are ms.
        timestamp: SECONDS * 1000,
        offline: true,
        caller_username: Some("caller.name".to_string()),
        video_orientation: Some(2),
        group: Some(wire_group_update()),
        action: Some(Action::Accept(pb::CallAcceptance {
            audio: wire_codecs(),
        })),
    };
    assert_eq!(call_event_of(&call), want);
}

#[test]
fn absent_optionals_stay_unset() {
    let call = bare_call(CallAction::Terminate {
        call_id: CALL_ID.to_string(),
        call_creator: lib(CALLER),
        reason: None,
        duration: None,
        audio_duration: None,
    });
    let want = bare_wire(Action::Terminate(pb::CallTermination {
        reason: None,
        duration: None,
        audio_duration: None,
    }));
    assert_eq!(call_event_of(&call), want);
}

#[test]
fn an_empty_group_call_snapshot_keeps_its_absent_fields_unset() {
    let update = GroupCallUpdate::builder()
        .call_id(CALL_ID.to_string())
        .call_creator(lib(CALLER))
        .transaction_id(0)
        .media(String::new())
        .connected_limit(0)
        .joinable(false)
        .av_upgradable(false)
        .rekey_requested(false)
        .participants(vec![GroupCallParticipant::new(
            lib(MEMBER),
            vec![GroupCallDevice::new(lib(MEMBER_DEVICE))],
        )])
        .build();
    let call = bare_call(CallAction::GroupUpdate {
        update: Box::new(update),
    });
    let want = bare_wire(Action::GroupUpdate(pb::GroupCallUpdate {
        call_id: CALL_ID.to_string(),
        call_creator: wire(CALLER),
        participants: vec![pb::GroupCallParticipant {
            jid: wire(MEMBER),
            devices: vec![pb::GroupCallDevice {
                jid: wire(MEMBER_DEVICE),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }));
    assert_eq!(call_event_of(&call), want);
}
