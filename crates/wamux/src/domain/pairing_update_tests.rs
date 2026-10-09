//! #150 (part 3 of #141): the pairing events past QR, code, success and
//! error. The mappings go through `map_event`, so each test also proves the
//! event no longer falls into the RawEvent catch-all; the stream's end is the
//! pure rule PairWithQr and PairWithCode apply to every update.

use std::time::Duration;

use wacore::pair_code::PairCodeRejection;
use wacore::types::events::{Event, PairingCodeError, PairingCodeRefresh, PairingQrCodesExhausted};

use super::pairing_stream_ends;
use crate::domain::event_mapping::map_event;
use crate::proto::v1 as pb;
use crate::proto::v1::event_envelope::Event as PbEvent;
use crate::proto::v1::pairing_update::Event as Pairing;

/// The one PairingUpdate case the event maps to; anything else is a failure.
fn pairing_of(event: Event) -> Pairing {
    match map_event(&event).as_slice() {
        [PbEvent::Pairing(pb::PairingUpdate { event: Some(p) })] => p.clone(),
        other => panic!("expected one PairingUpdate, got {other:?}"),
    }
}

fn code_error_of(error: PairingCodeError) -> pb::PairingCodeErrorInfo {
    match pairing_of(Event::PairingCodeError(error)) {
        Pairing::CodeError(info) => info,
        other => panic!("expected code_error, got {other:?}"),
    }
}

fn update(event: Pairing) -> pb::PairingUpdate {
    pb::PairingUpdate { event: Some(event) }
}

#[test]
fn pairing_code_refresh_relays_force_manual() {
    for force_manual in [true, false] {
        let event = Event::PairingCodeRefresh(
            PairingCodeRefresh::builder()
                .force_manual(force_manual)
                .build(),
        );
        assert_eq!(
            pairing_of(event),
            Pairing::CodeRefresh(pb::PairingCodeRefreshInfo { force_manual })
        );
    }
}

/// Every refusal the library names, to its own case and no code; a number it
/// does not name crosses as UNKNOWN with the number (#73).
#[test]
fn pairing_code_error_maps_every_rejection() {
    use pb::PairCodeRejection as R;
    let cases = [
        (PairCodeRejection::BadRequest, R::BadRequest, 0),
        (PairCodeRejection::Forbidden, R::Forbidden, 0),
        (PairCodeRejection::RateOverlimit, R::RateOverlimit, 0),
        (
            PairCodeRejection::FeatureNotAvailable,
            R::FeatureNotAvailable,
            0,
        ),
        (
            PairCodeRejection::InternalServerError,
            R::InternalServerError,
            0,
        ),
        (PairCodeRejection::Unknown(460), R::Unknown, 460),
    ];
    for (library, wire, code) in cases {
        let info = code_error_of(
            PairingCodeError::builder()
                .rejection(library)
                .error("refused".to_string())
                .build(),
        );
        assert_eq!(
            (info.rejection, info.rejection_code),
            (wire as i32, code),
            "{library:?}"
        );
    }
}

/// A local failure (a phone number too short never reaches the server) has no
/// refusal: UNSPECIFIED, no backoff, and the library's text as `detail`.
#[test]
fn pairing_code_error_without_rejection_is_unspecified() {
    let info = code_error_of(
        PairingCodeError::builder()
            .error("phone number too short".to_string())
            .build(),
    );
    let expected = pb::PairingCodeErrorInfo {
        rejection: pb::PairCodeRejection::Unspecified as i32,
        rejection_code: 0,
        backoff_ms: None,
        detail: "phone number too short".to_string(),
    };
    assert_eq!(info, expected);
}

#[test]
fn pairing_code_error_backoff_crosses_in_ms() {
    let info = code_error_of(
        PairingCodeError::builder()
            .rejection(PairCodeRejection::RateOverlimit)
            .backoff(Duration::from_millis(30_500))
            .error("rate-overlimit".to_string())
            .build(),
    );
    assert_eq!(info.backoff_ms, Some(30_500));
}

#[test]
fn pairing_qr_codes_exhausted_relays_disconnected() {
    for disconnected in [true, false] {
        let event = Event::PairingQrCodesExhausted(
            PairingQrCodesExhausted::builder()
                .disconnected(disconnected)
                .build(),
        );
        assert_eq!(
            pairing_of(event),
            Pairing::QrCodesExhausted(pb::PairingQrCodesExhaustedInfo { disconnected })
        );
    }
}

/// Success, failure, a refused or withdrawn code, and QR refs used up with the
/// connection closed: nothing more can come on the flow, so the stream ends.
#[test]
fn pairing_stream_ends_when_nothing_more_can_come() {
    let terminal = [
        Pairing::Paired(pb::PairedInfo::default()),
        Pairing::Error(pb::PairingError::default()),
        Pairing::CodeError(pb::PairingCodeErrorInfo::default()),
        Pairing::CodeRefresh(pb::PairingCodeRefreshInfo {
            force_manual: false,
        }),
        Pairing::QrCodesExhausted(pb::PairingQrCodesExhaustedInfo { disconnected: true }),
    ];
    for event in terminal {
        assert!(pairing_stream_ends(&update(event.clone())), "{event:?}");
    }
}

/// A QR, a code, and QR refs used up while a pair code is still valid on the
/// same connection (`disconnected` false): the flow goes on. An update with no
/// case at all ends nothing either.
#[test]
fn pairing_stream_continues_on_progress() {
    let progress = [
        Pairing::QrCode("2@abc".to_string()),
        Pairing::PairCode("ABCD-EFGH".to_string()),
        Pairing::QrCodesExhausted(pb::PairingQrCodesExhaustedInfo {
            disconnected: false,
        }),
    ];
    for event in progress {
        assert!(!pairing_stream_ends(&update(event.clone())), "{event:?}");
    }
    assert!(!pairing_stream_ends(&pb::PairingUpdate { event: None }));
}
