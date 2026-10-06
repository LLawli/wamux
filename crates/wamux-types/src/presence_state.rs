//! What SendPresence raises (#127): the account's own presence, or a chat state
//! in one chat. A closed enum the domain matches with no wildcard arm.

use wamux_proto::v1 as pb;

use crate::error::WamuxError;
use crate::request_enum::{refusal, shown_value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PresenceState {
    Available,
    Unavailable,
    Composing,
    Recording,
    Paused,
}

impl PresenceState {
    /// Every state, for tests and value tables.
    pub const ALL: [PresenceState; 5] = [
        Self::Available,
        Self::Unavailable,
        Self::Composing,
        Self::Recording,
        Self::Paused,
    ];

    /// `InvalidArgument("state must be one of PRESENCE_STATE_AVAILABLE|...|PRESENCE_STATE_PAUSED, got <v>")`
    /// for UNSPECIFIED, UNKNOWN or a number outside the enum, `<v>` the
    /// value's proto name or the bare number.
    pub fn parse(value: i32) -> Result<Self, WamuxError> {
        let parsed = match pb::PresenceState::try_from(value) {
            Ok(pb::PresenceState::Available) => Some(Self::Available),
            Ok(pb::PresenceState::Unavailable) => Some(Self::Unavailable),
            Ok(pb::PresenceState::Composing) => Some(Self::Composing),
            Ok(pb::PresenceState::Recording) => Some(Self::Recording),
            Ok(pb::PresenceState::Paused) => Some(Self::Paused),
            // No wildcard: a variant added to the proto breaks the build here.
            Ok(pb::PresenceState::Unspecified | pb::PresenceState::Unknown) | Err(_) => None,
        };
        parsed.ok_or_else(|| refuse_state(value))
    }

    /// The `state` value on the wire.
    pub fn to_wire(self) -> pb::PresenceState {
        match self {
            Self::Available => pb::PresenceState::Available,
            Self::Unavailable => pb::PresenceState::Unavailable,
            Self::Composing => pb::PresenceState::Composing,
            Self::Recording => pb::PresenceState::Recording,
            Self::Paused => pb::PresenceState::Paused,
        }
    }
}

fn refuse_state(value: i32) -> WamuxError {
    let name = pb::PresenceState::try_from(value)
        .ok()
        .map(|wire| wire.as_str_name());
    let shown = shown_value(name, value);
    let names = PresenceState::ALL
        .iter()
        .map(|state| state.to_wire().as_str_name());
    refusal("state", names, &shown)
}
