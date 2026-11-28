//! #135: one hand-built value per `CallAction` variant (14), every field away
//! from its default, for the actions two personal phones cannot provoke (group
//! calls, call links, screen share).

use std::collections::HashSet;
use std::mem::discriminant;

use wacore::types::call::{CallAction, VideoState};
use wacore::types::group_call::{ScreenShare, ScreenShareState};

use super::tests::{
    CALL_ID, CALLER, CALLER_PN, GROUP, bare_call, bare_wire, codecs, enc_rekey, group_update, lib,
    waiting_room, wire, wire_codecs, wire_enc_rekey, wire_group_update, wire_waiting_room,
};
use super::*;
use crate::proto::v1::call_event::Action;

#[expect(
    clippy::too_many_lines,
    reason = "fixture table: one hand-built value per signaling CallAction variant and its wire case"
)]
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

#[expect(
    clippy::too_many_lines,
    reason = "fixture table: one hand-built value per in-call CallAction variant and its wire case"
)]
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
