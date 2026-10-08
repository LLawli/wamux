//! #135: every call enum value the library names maps to its own contract
//! value; a `<video state>` number it does not name lands in UNKNOWN with the
//! number.

use wacore::types::call::VideoState;
use wacore::types::group_call::{CallLinkMedia, ScreenShareState};
use wamux_proto::v1 as pb;

use crate::call_enums::{call_link_media_of, screen_share_state_of, video_state_of};

#[test]
fn every_video_state_maps_to_its_value() {
    use pb::CallVideoStateKind as P;
    let table = [
        (VideoState::Disabled, P::Disabled),
        (VideoState::Enabled, P::Enabled),
        (VideoState::Paused, P::Paused),
        (VideoState::UpgradeRequest, P::UpgradeRequest),
        (VideoState::UpgradeAccept, P::UpgradeAccept),
        (VideoState::UpgradeReject, P::UpgradeReject),
        (VideoState::Stopped, P::Stopped),
        (
            VideoState::UpgradeRejectByTimeout,
            P::UpgradeRejectByTimeout,
        ),
        (VideoState::UpgradeCancel, P::UpgradeCancel),
        (
            VideoState::UpgradeCancelByTimeout,
            P::UpgradeCancelByTimeout,
        ),
        (VideoState::UnknownPeer, P::UnknownPeer),
        (VideoState::UpgradeRequestV2, P::UpgradeRequestV2),
        (VideoState::Error, P::Error),
    ];
    for (library, wire) in table {
        // A named state carries no code: the enum says it all.
        assert_eq!(video_state_of(library), (wire, 0), "{library:?}");
    }
}

#[test]
fn a_video_state_number_the_library_does_not_name_is_unknown_with_it() {
    assert_eq!(
        video_state_of(VideoState::Unknown(42)),
        (pb::CallVideoStateKind::Unknown, 42)
    );
}

#[test]
fn a_named_video_state_is_never_unknown() {
    // The library's own int parse: every number it names comes back named.
    for code in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 20] {
        let (wire, raw) = video_state_of(VideoState::from(code));
        assert_ne!(wire, pb::CallVideoStateKind::Unknown, "state {code}");
        assert_eq!(raw, 0, "state {code}");
    }
}

#[test]
fn every_screen_share_state_maps_to_its_value() {
    assert_eq!(
        screen_share_state_of(ScreenShareState::Started),
        pb::ScreenShareState::Started
    );
    assert_eq!(
        screen_share_state_of(ScreenShareState::Stopped),
        pb::ScreenShareState::Stopped
    );
}

#[test]
fn every_call_link_media_maps_to_its_value() {
    assert_eq!(
        call_link_media_of(CallLinkMedia::Audio),
        pb::CallLinkMedia::Audio
    );
    assert_eq!(
        call_link_media_of(CallLinkMedia::Video),
        pb::CallLinkMedia::Video
    );
}
