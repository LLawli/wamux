//! #135: the typed projection of a library `IncomingCall`, asserted as the
//! whole message. One hand-built value per `CallAction` variant (14), every
//! field away from its default, for the actions two personal phones cannot
//! provoke (group calls, call links, screen share); the live captures are in
//! `call_event_capture_tests.rs`.

use std::collections::HashSet;
use std::mem::discriminant;
use std::str::FromStr;

use wacore::types::call::{CallAction, CallAudioCodec, IncomingCall, VideoState};
use wacore::types::group_call::{
    CallLinkMedia, GroupCallDevice, GroupCallEncRekey, GroupCallParticipant, GroupCallRelay,
    GroupCallRelayEndpoint, GroupCallUpdate, ScreenShare, ScreenShareState, WaitingRoom,
    WaitingRoomUser,
};
use whatsapp_rust::Jid;

use super::*;
use crate::proto::v1::call_event::Action;

const CALLER: &str = "100000000000001@lid";
const CALLER_PN: &str = "5511900000001@s.whatsapp.net";
const MEMBER: &str = "100000000000002@lid";
const MEMBER_PN: &str = "5511900000002@s.whatsapp.net";
const MEMBER_DEVICE: &str = "100000000000002:3@lid";
const GROUP: &str = "120363000000000135@g.us";
const CALL_ID: &str = "CALL-135";
const STANZA_ID: &str = "STANZA-135";
const SECONDS: i64 = 1_791_457_942;

fn lib(value: &str) -> Jid {
    Jid::from_str(value).expect("test jid parses")
}

fn wire(value: &str) -> Option<pb::Jid> {
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

fn codecs() -> Vec<CallAudioCodec> {
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

fn wire_codecs() -> Vec<pb::CallAudioCodec> {
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

fn group_update() -> GroupCallUpdate {
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

fn wire_group_update() -> pb::GroupCallUpdate {
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

fn enc_rekey() -> GroupCallEncRekey {
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

fn wire_enc_rekey() -> pb::GroupCallEncRekey {
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

fn waiting_room() -> WaitingRoom {
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

fn wire_waiting_room() -> pb::CallWaitingRoom {
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

// ---------------------------------------------------------------------------
// One value per CallAction variant
// ---------------------------------------------------------------------------

fn signaling_actions() -> Vec<(CallAction, Action)> {
    let id = || CALL_ID.to_string();
    let creator = || lib(CALLER);
    vec![
        (
            CallAction::Offer {
                call_id: id(),
                call_creator: creator(),
                caller_pn: Some(lib(CALLER_PN)),
                caller_country_code: Some("BR".to_string()),
                device_class: Some("2016".to_string()),
                joinable: true,
                is_video: true,
                audio: codecs(),
                group_jid: Some(lib(GROUP)),
            },
            Action::Offer(pb::CallOffer {
                caller_pn: wire(CALLER_PN),
                caller_country_code: Some("BR".to_string()),
                device_class: Some("2016".to_string()),
                joinable: true,
                is_video: true,
                audio: wire_codecs(),
                group_jid: wire(GROUP),
            }),
        ),
        (
            CallAction::OfferNotice {
                call_id: id(),
                call_creator: creator(),
                is_video: true,
                is_group: true,
            },
            Action::OfferNotice(pb::CallOfferNotice {
                is_video: true,
                is_group: true,
            }),
        ),
        (
            CallAction::PreAccept {
                call_id: id(),
                call_creator: creator(),
                audio: codecs(),
            },
            Action::PreAccept(pb::CallAcceptance {
                audio: wire_codecs(),
            }),
        ),
        (
            CallAction::Accept {
                call_id: id(),
                call_creator: creator(),
                audio: codecs(),
            },
            Action::Accept(pb::CallAcceptance {
                audio: wire_codecs(),
            }),
        ),
        (
            CallAction::Reject {
                call_id: id(),
                call_creator: creator(),
                reason: Some("busy".to_string()),
            },
            Action::Reject(pb::CallRejection {
                reason: Some("busy".to_string()),
            }),
        ),
        (
            CallAction::Terminate {
                call_id: id(),
                call_creator: creator(),
                reason: Some("timeout".to_string()),
                // Verbatim: the unit is not verified.
                duration: Some(3670),
                audio_duration: Some(3600),
            },
            Action::Terminate(pb::CallTermination {
                reason: Some("timeout".to_string()),
                duration: Some(3670),
                audio_duration: Some(3600),
            }),
        ),
        (
            CallAction::Transport {
                call_id: id(),
                call_creator: creator(),
                p2p_cand_round: Some("2".to_string()),
                transport_message_type: Some("3".to_string()),
            },
            Action::Transport(pb::CallTransport {
                p2p_cand_round: Some("2".to_string()),
                transport_message_type: Some("3".to_string()),
            }),
        ),
    ]
}

fn in_call_actions() -> Vec<(CallAction, Action)> {
    let id = || CALL_ID.to_string();
    let creator = || lib(CALLER);
    let screen_share = ScreenShare::builder()
        .state(ScreenShareState::Started)
        .version(3)
        .screen_share_id(77)
        .build();
    vec![
        (
            CallAction::RelayLatency {
                call_id: id(),
                call_creator: creator(),
            },
            Action::RelayLatency(pb::Empty {}),
        ),
        (
            CallAction::VideoState {
                call_id: id(),
                call_creator: creator(),
                state: VideoState::UpgradeRequest,
                orientation: Some(3),
                dec: Some("H264,AV1".to_string()),
            },
            Action::VideoState(pb::CallVideoState {
                state: pb::CallVideoStateKind::UpgradeRequest as i32,
                state_code: 0,
                orientation: Some(3),
                dec: Some("H264,AV1".to_string()),
            }),
        ),
        (
            CallAction::GroupUpdate {
                update: Box::new(group_update()),
            },
            Action::GroupUpdate(wire_group_update()),
        ),
        (
            CallAction::EncRekey {
                rekey: Box::new(enc_rekey()),
            },
            Action::EncRekey(wire_enc_rekey()),
        ),
        (
            CallAction::WaitingRoomUpdate {
                room: Box::new(waiting_room()),
            },
            Action::WaitingRoomUpdate(wire_waiting_room()),
        ),
        (
            CallAction::RaiseHand {
                call_id: id(),
                call_creator: creator(),
                raised: true,
            },
            Action::RaiseHand(pb::CallRaiseHand { raised: true }),
        ),
        (
            CallAction::ScreenShare {
                call_id: id(),
                call_creator: creator(),
                screen_share,
            },
            Action::ScreenShare(pb::CallScreenShare {
                state: pb::ScreenShareState::Started as i32,
                version: 3,
                screen_share_id: Some(77),
            }),
        ),
    ]
}

fn every_action() -> Vec<(CallAction, Action)> {
    let mut all = signaling_actions();
    all.extend(in_call_actions());
    all
}

/// The minimal call the library's test constructor builds around `action`.
fn bare_call(action: CallAction) -> IncomingCall {
    IncomingCall::new_for_test(lib(CALLER), STANZA_ID.to_string(), at!(SECONDS), action)
}

/// What a bare call maps to around `action`: every optional unset.
fn bare_wire(action: Action) -> pb::CallEvent {
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
fn every_call_action_maps_to_its_case() {
    let table = every_action();
    let cases: HashSet<_> = table.iter().map(|(_, wire)| discriminant(wire)).collect();
    assert_eq!(
        cases.len(),
        14,
        "one case per CallAction variant at the pinned rev"
    );
    for (library, want) in table {
        let tag = library.wire_tag();
        assert_eq!(call_event_of(&bare_call(library)), bare_wire(want), "{tag}");
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
