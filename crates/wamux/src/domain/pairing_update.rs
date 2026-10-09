//! The pairing events past QR, code, success and error (#150, part 3 of
//! #141): a pair code withdrawn, a pair code refused and the QR refs used up.
//! They reached the socket as RawEvent, and never reached the PairWithQr and
//! PairWithCode streams at all.

use wacore::pair_code::PairCodeRejection;
use wacore::types::events::{PairingCodeError, PairingCodeRefresh, PairingQrCodesExhausted};

use crate::proto::v1 as pb;

pub fn pairing_code_refresh_of(refresh: &PairingCodeRefresh) -> pb::pairing_update::Event {
    pb::pairing_update::Event::CodeRefresh(pb::PairingCodeRefreshInfo {
        force_manual: refresh.force_manual,
    })
}

pub fn pairing_code_error_of(error: &PairingCodeError) -> pb::pairing_update::Event {
    let (rejection, rejection_code) = pair_code_rejection_of(error.rejection.as_ref());
    pb::pairing_update::Event::CodeError(pb::PairingCodeErrorInfo {
        rejection: rejection as i32,
        rejection_code,
        // Saturating: a Duration past i64 ms is not a realistic server wait.
        backoff_ms: error
            .backoff
            .map(|wait| i64::try_from(wait.as_millis()).unwrap_or(i64::MAX)),
        detail: error.error.clone(),
    })
}

/// The rejection and the server's number when UNKNOWN (0 otherwise). Closed
/// set, no wildcard arm, so a new library variant breaks the build (#126).
fn pair_code_rejection_of(rejection: Option<&PairCodeRejection>) -> (pb::PairCodeRejection, i32) {
    let known = match rejection {
        None => pb::PairCodeRejection::Unspecified,
        Some(PairCodeRejection::BadRequest) => pb::PairCodeRejection::BadRequest,
        Some(PairCodeRejection::Forbidden) => pb::PairCodeRejection::Forbidden,
        Some(PairCodeRejection::RateOverlimit) => pb::PairCodeRejection::RateOverlimit,
        Some(PairCodeRejection::FeatureNotAvailable) => pb::PairCodeRejection::FeatureNotAvailable,
        Some(PairCodeRejection::InternalServerError) => pb::PairCodeRejection::InternalServerError,
        Some(PairCodeRejection::Unknown(code)) => {
            return (pb::PairCodeRejection::Unknown, *code);
        }
    };
    (known, 0)
}

pub fn pairing_qr_codes_exhausted_of(
    exhausted: &PairingQrCodesExhausted,
) -> pb::pairing_update::Event {
    pb::pairing_update::Event::QrCodesExhausted(pb::PairingQrCodesExhaustedInfo {
        disconnected: exhausted.disconnected,
    })
}

/// Whether a pairing stream is over after this update: nothing more can come
/// on its flow. Exhausted QR refs with the connection still up leave a live
/// pair code, so that one keeps going.
pub fn pairing_stream_ends(update: &pb::PairingUpdate) -> bool {
    use pb::pairing_update::Event;
    match &update.event {
        Some(Event::Paired(_) | Event::Error(_) | Event::CodeError(_) | Event::CodeRefresh(_)) => {
            true
        }
        Some(Event::QrCodesExhausted(info)) => info.disconnected,
        Some(Event::QrCode(_) | Event::PairCode(_)) | None => false,
    }
}

#[cfg(test)]
#[path = "pairing_update_tests.rs"]
mod tests;
