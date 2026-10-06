//! `PresenceState` (#127): every value SendPresence takes, by name, and one
//! refusal for every way to be wrong.

use wamux_proto::v1 as pb;

use crate::{PresenceState, WamuxError};

const WIRE: [(PresenceState, pb::PresenceState); 5] = [
    (PresenceState::Available, pb::PresenceState::Available),
    (PresenceState::Unavailable, pb::PresenceState::Unavailable),
    (PresenceState::Composing, pb::PresenceState::Composing),
    (PresenceState::Recording, pb::PresenceState::Recording),
    (PresenceState::Paused, pb::PresenceState::Paused),
];

const REFUSAL: &str = "state must be one of PRESENCE_STATE_AVAILABLE|\
PRESENCE_STATE_UNAVAILABLE|PRESENCE_STATE_COMPOSING|PRESENCE_STATE_RECORDING|\
PRESENCE_STATE_PAUSED, got";

fn invalid_argument(result: Result<PresenceState, WamuxError>) -> String {
    match result {
        Err(WamuxError::InvalidArgument(message)) => message,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

#[test]
fn every_presence_state_parses_from_its_wire_value() {
    for (state, wire) in WIRE {
        assert_eq!(PresenceState::parse(wire as i32).unwrap(), state);
    }
    assert_eq!(PresenceState::ALL.len(), WIRE.len());
}

#[test]
fn an_unset_unknown_or_unlisted_presence_state_is_refused() {
    let refused = [
        (0, "PRESENCE_STATE_UNSPECIFIED"),
        (1, "PRESENCE_STATE_UNKNOWN"),
        (42, "42"),
        (-1, "-1"),
    ];
    for (value, shown) in refused {
        assert_eq!(
            invalid_argument(PresenceState::parse(value)),
            format!("{REFUSAL} {shown}")
        );
    }
}

#[test]
fn every_presence_state_round_trips() {
    for (state, wire) in WIRE {
        assert_eq!(state.to_wire(), wire, "{state:?}");
    }
    for state in PresenceState::ALL {
        assert_eq!(PresenceState::parse(state.to_wire() as i32).unwrap(), state);
    }
}
