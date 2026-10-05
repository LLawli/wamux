//! `send_echo` (#115): the tests that lived inline, moved so the loop can seal
//! them. Unchanged: they never took a `pb::*` input.

use super::fits_the_ring;
use crate::proto::v1 as pb;
use crate::proto::v1::event_envelope::Event as WireEvent;

fn envelope(payload_len: usize) -> pb::EventEnvelope {
    pb::EventEnvelope {
        account_uuid: "a".to_string(),
        monotonic_seq: 0,
        ts_unix_ms: 0,
        event: Some(WireEvent::Message(pb::InboundMessage {
            raw_message: vec![0u8; payload_len],
            ..Default::default()
        })),
    }
}

#[test]
fn no_cap_keeps_every_echo() {
    assert!(fits_the_ring(&envelope(4096), 0));
}

// The echo answers to the same ring budget as a relayed event; a huge one
// must not evict the live history a reconnect depends on.
#[test]
fn an_echo_over_the_cap_stays_out_of_the_ring() {
    assert!(!fits_the_ring(&envelope(4096), 64));
    assert!(fits_the_ring(&envelope(8), 64));
}
