//! An inbound `<call>` (whatsapp-rust `IncomingCall`) as the typed `CallEvent`
//! (#135, part 4 of #74). It used to cross as the library's JSON in `raw`.
//! Every public field crosses, including the three the library keeps out of
//! its own JSON (a participant's `pn`, a device's capability blob, a rekey's
//! ciphertext): the relay does not pick. One concrete function per library
//! struct, no generic.

use wacore::types::call::{CallAction, CallAudioCodec, IncomingCall, VideoState};
use wacore::types::group_call::{
    GroupCallDevice, GroupCallEncRekey, GroupCallParticipant, GroupCallRelay,
    GroupCallRelayEndpoint, GroupCallUpdate, ScreenShare, WaitingRoom, WaitingRoomUser,
};
use wamux_types::call_enums::{call_link_media_of, screen_share_state_of, video_state_of};

use crate::domain::group_metadata::{lib_jid, optional_lib_jid};
use crate::proto::v1 as pb;
use crate::proto::v1::call_event::Action;

/// The typed event for one inbound call stanza.
pub fn call_event_of(call: &IncomingCall) -> pb::CallEvent {
    pb::CallEvent {
        from: lib_jid(&call.from),
        call_id: call.action.call_id().to_string(),
        call_creator: lib_jid(call.action.call_creator()),
        stanza_id: call.stanza_id.clone(),
        notify: call.notify.clone(),
        platform: call.platform.clone(),
        version: call.version.clone(),
        participant: optional_lib_jid(call.participant.as_ref()),
        recipient: optional_lib_jid(call.recipient.as_ref()),
        // The stanza's `t` is unix seconds; the contract's instants are ms.
        timestamp: call.timestamp.timestamp_millis(),
        offline: call.offline,
        caller_username: call.caller_username.clone(),
        video_orientation: call.video_orientation.map(u32::from),
        group: call.group.as_deref().map(group_update),
        action: Some(action_of(&call.action)),
    }
}

fn codecs(audio: &[CallAudioCodec]) -> Vec<pb::CallAudioCodec> {
    audio
        .iter()
        .map(|codec| pb::CallAudioCodec {
            enc: codec.enc.clone(),
            rate: codec.rate,
        })
        .collect()
}

/// One case per `CallAction` variant. The id and creator live on the event
/// itself, so each payload carries only what is its own. The match is split
/// across four small functions, each handing what it does not know to the
/// next; the last one owns the wildcard.
fn action_of(action: &CallAction) -> Action {
    match action {
        CallAction::Offer {
            caller_pn,
            caller_country_code,
            device_class,
            joinable,
            is_video,
            audio,
            group_jid,
            ..
        } => Action::Offer(pb::CallOffer {
            caller_pn: optional_lib_jid(caller_pn.as_ref()),
            caller_country_code: caller_country_code.clone(),
            device_class: device_class.clone(),
            joinable: *joinable,
            is_video: *is_video,
            audio: codecs(audio),
            group_jid: optional_lib_jid(group_jid.as_ref()),
        }),
        CallAction::OfferNotice {
            is_video, is_group, ..
        } => Action::OfferNotice(pb::CallOfferNotice {
            is_video: *is_video,
            is_group: *is_group,
        }),
        _ => answer_or_end_action(action),
    }
}

fn answer_or_end_action(action: &CallAction) -> Action {
    match action {
        CallAction::PreAccept { audio, .. } => Action::PreAccept(acceptance(audio)),
        CallAction::Accept { audio, .. } => Action::Accept(acceptance(audio)),
        CallAction::Reject { reason, .. } => Action::Reject(pb::CallRejection {
            reason: reason.clone(),
        }),
        CallAction::Terminate {
            reason,
            duration,
            audio_duration,
            ..
        } => Action::Terminate(pb::CallTermination {
            reason: reason.clone(),
            // Verbatim: the unit is not verified.
            duration: *duration,
            audio_duration: *audio_duration,
        }),
        _ => media_signal_action(action),
    }
}

fn acceptance(audio: &[CallAudioCodec]) -> pb::CallAcceptance {
    pb::CallAcceptance {
        audio: codecs(audio),
    }
}

fn media_signal_action(action: &CallAction) -> Action {
    match action {
        CallAction::Transport {
            p2p_cand_round,
            transport_message_type,
            ..
        } => Action::Transport(pb::CallTransport {
            p2p_cand_round: p2p_cand_round.clone(),
            transport_message_type: transport_message_type.clone(),
        }),
        CallAction::RelayLatency { .. } => Action::RelayLatency(pb::Empty {}),
        CallAction::VideoState {
            state,
            orientation,
            dec,
            ..
        } => Action::VideoState(video_state(*state, *orientation, dec)),
        CallAction::RaiseHand { raised, .. } => {
            Action::RaiseHand(pb::CallRaiseHand { raised: *raised })
        }
        _ => group_signal_action(action),
    }
}

fn group_signal_action(action: &CallAction) -> Action {
    match action {
        CallAction::GroupUpdate { update } => Action::GroupUpdate(group_update(update)),
        CallAction::EncRekey { rekey } => Action::EncRekey(enc_rekey(rekey)),
        CallAction::WaitingRoomUpdate { room } => Action::WaitingRoomUpdate(waiting_room(room)),
        CallAction::ScreenShare { screen_share, .. } => {
            Action::ScreenShare(screen_share_payload(screen_share))
        }
        // `CallAction` is #[non_exhaustive]: a variant the library adds later
        // still reaches the edge, by the wire tag the library names it with.
        other => Action::UnknownAction(other.wire_tag().to_string()),
    }
}

fn video_state(
    state: VideoState,
    orientation: Option<u8>,
    dec: &Option<String>,
) -> pb::CallVideoState {
    let (kind, code) = video_state_of(state);
    pb::CallVideoState {
        state: kind as i32,
        state_code: code,
        orientation: orientation.map(u32::from),
        dec: dec.clone(),
    }
}

fn screen_share_payload(share: &ScreenShare) -> pb::CallScreenShare {
    pb::CallScreenShare {
        state: screen_share_state_of(share.state) as i32,
        version: share.version,
        screen_share_id: share.screen_share_id,
    }
}

fn group_update(update: &GroupCallUpdate) -> pb::GroupCallUpdate {
    pb::GroupCallUpdate {
        call_id: update.call_id.clone(),
        call_creator: lib_jid(&update.call_creator),
        group_jid: optional_lib_jid(update.group_jid.as_ref()),
        transaction_id: update.transaction_id,
        media: update.media.clone(),
        connected_limit: update.connected_limit,
        joinable: update.joinable,
        av_upgradable: update.av_upgradable,
        rekey_requested: update.rekey_requested,
        participants: update.participants.iter().map(participant).collect(),
        relay: update.relay.as_ref().map(relay),
    }
}

fn participant(participant: &GroupCallParticipant) -> pb::GroupCallParticipant {
    pb::GroupCallParticipant {
        jid: lib_jid(&participant.jid),
        // The library keeps `pn` out of its JSON; the contract does not.
        pn: optional_lib_jid(participant.pn.as_ref()),
        state: participant.state.clone(),
        participant_type: participant.participant_type.clone(),
        devices: participant.devices.iter().map(device).collect(),
    }
}

fn device(device: &GroupCallDevice) -> pb::GroupCallDevice {
    pb::GroupCallDevice {
        jid: lib_jid(&device.jid),
        platform: device.platform.clone(),
        pid: device.pid,
        capability_version: device.capability_version,
        // `capability` is pub(crate) upstream; the accessor is the way in.
        capability: device.capability().to_vec(),
    }
}

/// The keys and tokens of the relay are private to the library, so only the
/// public half crosses.
fn relay(relay: &GroupCallRelay) -> pb::GroupCallRelay {
    pb::GroupCallRelay {
        transaction_id: relay.transaction_id,
        self_pid: relay.self_pid,
        uuid: relay.uuid.clone(),
        participant_uuid: relay.participant_uuid.clone(),
        attribute_padding: relay.attribute_padding,
        warp_mi_tag_len: relay.warp_mi_tag_len,
        endpoints: relay.endpoints.iter().map(relay_endpoint).collect(),
    }
}

fn relay_endpoint(endpoint: &GroupCallRelayEndpoint) -> pb::GroupCallRelayEndpoint {
    pb::GroupCallRelayEndpoint {
        relay_id: endpoint.relay_id,
        token_id: endpoint.token_id,
        auth_token_id: endpoint.auth_token_id,
        relay_name: endpoint.relay_name.clone(),
        domain_name: endpoint.domain_name.clone(),
        rtt_ms: endpoint.rtt_ms,
        is_fna: endpoint.is_fna,
        ipv4: endpoint.ipv4.clone(),
        port: endpoint.port.map(u32::from),
    }
}

fn enc_rekey(rekey: &GroupCallEncRekey) -> pb::GroupCallEncRekey {
    pb::GroupCallEncRekey {
        call_id: rekey.call_id.clone(),
        call_creator: lib_jid(&rekey.call_creator),
        transaction_id: rekey.transaction_id,
        key_generation: rekey.key_generation,
        encryption_type: rekey.encryption_type.clone(),
        encryption_version: rekey.encryption_version,
        ciphertext: rekey.ciphertext.clone(),
    }
}

fn waiting_room(room: &WaitingRoom) -> pb::CallWaitingRoom {
    pb::CallWaitingRoom {
        call_id: room.call_id.clone(),
        call_creator: lib_jid(&room.call_creator),
        link_token: room.link_token.clone(),
        media: call_link_media_of(room.media) as i32,
        enabled: room.enabled,
        is_admin: room.is_admin,
        transaction_id: room.transaction_id,
        users: room.users.iter().map(waiting_room_user).collect(),
    }
}

fn waiting_room_user(user: &WaitingRoomUser) -> pb::WaitingRoomUser {
    pb::WaitingRoomUser {
        jid: lib_jid(&user.jid),
        pn: optional_lib_jid(user.pn.as_ref()),
        state: user.state.clone(),
    }
}

// Tests live in sibling files to keep each one under the 500-line rule.
#[cfg(test)]
#[path = "call_event_capture_tests.rs"]
mod capture_tests;
#[cfg(test)]
#[path = "call_event_tests.rs"]
mod tests;
