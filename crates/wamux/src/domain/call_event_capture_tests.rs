//! #135: the ten `<call>` stanzas captured live on 2026-10-08 (three calls from
//! `pessoal` to `trabalho`, seen by a linked device of the callee), sent
//! through the binary codec, parsed by the library's own `parse_call_stanza`
//! and mapped through `map_event`, as in production. Each one is asserted as
//! the whole `CallEvent`.

use wacore::stanza::call::parse_call_stanza;
use wacore::types::events::Event;

use crate::domain::event_mapping::map_event;
use crate::domain::test_xml::{attributes_sorted, node_of_xml, stanza_lines, through_the_wire};
use crate::proto::v1 as pb;
use crate::proto::v1::call_event::Action;

const CAPTURE: &str = include_str!("fixtures/captured-call-2026-10-08.xml");
const CALLER: &str = "100000000000001@lid";
const CALLEE_PHONE: &str = "100000000000002@lid";
const CALLER_PN: &str = "5511900000001@s.whatsapp.net";
const DECLINED: &str = "0087B7D0D14C4BD7AFA42B796D5FF957";
const ANSWERED: &str = "005B1399865349C6CF9F6A16BD8C7F07";
const VIDEO: &str = "00DBEDA3371C62FD1D3D25603F8D324A";

fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

fn opus(rate: u32) -> pb::CallAudioCodec {
    pb::CallAudioCodec {
        enc: "opus".to_string(),
        rate,
    }
}

/// What every captured stanza shares: the caller created each call, nothing
/// was replayed, no companion routing, no group, no username.
fn header(from: &str, call_id: &str, stanza_id: &str, seconds: i64) -> pb::CallEvent {
    pb::CallEvent {
        from: wire(from),
        call_id: call_id.to_string(),
        call_creator: wire(CALLER),
        stanza_id: stanza_id.to_string(),
        timestamp: seconds * 1000,
        ..Default::default()
    }
}

/// The caller's Android client (`smba`) sends the offers.
fn offer(call_id: &str, stanza_id: &str, seconds: i64, is_video: bool) -> pb::CallEvent {
    pb::CallEvent {
        notify: Some("Caller".to_string()),
        platform: Some("smba".to_string()),
        version: Some("2.26.39.72".to_string()),
        action: Some(Action::Offer(pb::CallOffer {
            caller_pn: wire(CALLER_PN),
            caller_country_code: Some("BR".to_string()),
            device_class: Some("2016".to_string()),
            joinable: true,
            is_video,
            audio: vec![opus(16000), opus(8000)],
            group_jid: None,
        })),
        ..header(CALLER, call_id, stanza_id, seconds)
    }
}

/// The callee's iPhone answers.
fn accept(call_id: &str, stanza_id: &str, seconds: i64) -> pb::CallEvent {
    pb::CallEvent {
        platform: Some("iphone".to_string()),
        version: Some("2.26.39.74".to_string()),
        action: Some(Action::Accept(pb::CallAcceptance {
            audio: vec![opus(16000)],
        })),
        ..header(CALLEE_PHONE, call_id, stanza_id, seconds)
    }
}

/// The caller dismisses this linked device once another device decided.
fn terminate(call_id: &str, stanza_id: &str, seconds: i64, reason: &str) -> pb::CallEvent {
    pb::CallEvent {
        action: Some(Action::Terminate(pb::CallTermination {
            reason: Some(reason.to_string()),
            duration: None,
            audio_duration: None,
        })),
        ..header(CALLER, call_id, stanza_id, seconds)
    }
}

/// The captures in arrival order, as the edge must see them.
fn expected() -> Vec<pb::CallEvent> {
    vec![
        offer(
            DECLINED,
            "BDDD8AD3628E50B5F57FE0EDAF8A6A19",
            1_791_457_942,
            false,
        ),
        pb::CallEvent {
            action: Some(Action::RelayLatency(pb::Empty {})),
            ..header(
                CALLER,
                DECLINED,
                "44FBAE05225543116A591E724584ADCC",
                1_791_457_944,
            )
        },
        // The callee's phone declining: no reason is the user's own decline.
        pb::CallEvent {
            action: Some(Action::Reject(pb::CallRejection { reason: None })),
            ..header(CALLEE_PHONE, DECLINED, "1791442394-67", 1_791_457_954)
        },
        terminate(
            DECLINED,
            "4EF146EBA6E7A98EDECE2BB849C8C1D8",
            1_791_457_954,
            "rejected_elsewhere",
        ),
        offer(
            ANSWERED,
            "437B03034F171F2CB37DCF8E54E9C9DD",
            1_791_457_964,
            false,
        ),
        accept(ANSWERED, "1791442394-76", 1_791_457_969),
        terminate(
            ANSWERED,
            "1D9DCDF98309D053D52E5CC95B5F01A6",
            1_791_457_969,
            "accepted_elsewhere",
        ),
        // A video offer announces the caller's camera rotation; the answer its own.
        pb::CallEvent {
            video_orientation: Some(0),
            ..offer(
                VIDEO,
                "ADE7E4457E5C1EE2023CEFE1C59AE7EF",
                1_791_457_995,
                true,
            )
        },
        pb::CallEvent {
            video_orientation: Some(1),
            ..accept(VIDEO, "1791442394-90", 1_791_457_999)
        },
        terminate(
            VIDEO,
            "196B4EC3D458D32BBC87DE54792141FA",
            1_791_457_999,
            "accepted_elsewhere",
        ),
    ]
}

#[test]
fn captured_calls_render_exactly_as_received() {
    let lines = stanza_lines(CAPTURE);
    assert_eq!(lines.len(), 10, "one per captured stanza");
    for line in lines {
        let rendered = wacore::xml::DisplayableNode(&node_of_xml(line)).to_string();
        assert_eq!(attributes_sorted(&rendered), attributes_sorted(line));
    }
}

#[test]
fn captured_calls_map_by_value() {
    let lines = stanza_lines(CAPTURE);
    let expected = expected();
    assert_eq!(lines.len(), expected.len(), "one expectation per capture");
    for (index, (line, want)) in lines.into_iter().zip(expected).enumerate() {
        let node = through_the_wire(&node_of_xml(line));
        let call = parse_call_stanza(&node.as_node_ref())
            .expect("the library parses the capture")
            .expect("a call action the library knows");
        let event = Event::IncomingCall(Box::new(call));
        let got = map_event(&event);
        assert_eq!(
            got,
            vec![pb::event_envelope::Event::Call(want)],
            "capture line {index}"
        );
    }
}
