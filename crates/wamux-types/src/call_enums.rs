//! The library's call enums as the contract's enums (#135, part 4 of #74).
//!
//! Same rules as `event_enums`: an explicit `match` per enum, written once.
//! `VideoState` has an `Unknown(code)` fallback, so it maps to UNKNOWN plus the
//! server's number; `ScreenShareState` and `CallLinkMedia` are closed, so their
//! matches have no wildcard and a variant added upstream fails the build.

use wacore::types::call::VideoState;
use wacore::types::group_call::{CallLinkMedia, ScreenShareState};
use wamux_proto::v1 as pb;

/// The `<video state=N>` value, and the server's number only when it is
/// UNKNOWN (0 otherwise).
pub fn video_state_of(state: VideoState) -> (pb::CallVideoStateKind, i32) {
    use pb::CallVideoStateKind as Kind;
    let kind = match state {
        VideoState::Unknown(code) => return (Kind::Unknown, code),
        VideoState::Disabled => Kind::Disabled,
        VideoState::Enabled => Kind::Enabled,
        VideoState::Paused => Kind::Paused,
        VideoState::UpgradeRequest => Kind::UpgradeRequest,
        VideoState::UpgradeAccept => Kind::UpgradeAccept,
        VideoState::UpgradeReject => Kind::UpgradeReject,
        VideoState::Stopped => Kind::Stopped,
        VideoState::UpgradeRejectByTimeout => Kind::UpgradeRejectByTimeout,
        VideoState::UpgradeCancel => Kind::UpgradeCancel,
        VideoState::UpgradeCancelByTimeout => Kind::UpgradeCancelByTimeout,
        VideoState::UnknownPeer => Kind::UnknownPeer,
        VideoState::UpgradeRequestV2 => Kind::UpgradeRequestV2,
        VideoState::Error => Kind::Error,
    };
    (kind, 0)
}

/// A participant's screen-share transition.
pub fn screen_share_state_of(state: ScreenShareState) -> pb::ScreenShareState {
    match state {
        ScreenShareState::Started => pb::ScreenShareState::Started,
        ScreenShareState::Stopped => pb::ScreenShareState::Stopped,
    }
}

/// A call link's media.
pub fn call_link_media_of(media: CallLinkMedia) -> pb::CallLinkMedia {
    match media {
        CallLinkMedia::Audio => pb::CallLinkMedia::Audio,
        CallLinkMedia::Video => pb::CallLinkMedia::Video,
    }
}
