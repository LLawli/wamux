//! The connection's lifecycle past its state (#150, part 3 of #141): a refused
//! connection, an outdated build, a stream error, a takeover, an incomplete
//! app-state sync and a build deadline. Each reached the socket as RawEvent
//! with the library's JSON.

use wacore::types::events::{
    AppStateSyncFailed, ClientExpirationChanged, ClientOutdated, ConnectFailure, StreamError,
    StreamReplaced,
};

use wamux_types::event_enums::logout_reason_of;

use crate::domain::stanza_node::stanza_node_of;
use crate::domain::wire_time::millis_from_signed_seconds;
use crate::proto::v1 as pb;

fn notice(notice: pb::connection_notice::Notice) -> pb::ConnectionNotice {
    pb::ConnectionNotice {
        notice: Some(notice),
    }
}

/// Reuses the logout reason mapping (#126): same server codes, same UNKNOWN rule.
pub fn connect_failure_notice_of(failure: &ConnectFailure) -> pb::ConnectionNotice {
    let (reason, reason_code) = logout_reason_of(failure.reason);
    notice(pb::connection_notice::Notice::ConnectFailure(
        pb::ConnectFailureInfo {
            reason: reason as i32,
            reason_code,
            message: failure.message.clone(),
            stanza: failure.raw.as_ref().map(stanza_node_of),
        },
    ))
}

pub fn client_outdated_notice_of(outdated: &ClientOutdated) -> pb::ConnectionNotice {
    notice(pb::connection_notice::Notice::ClientOutdated(
        pb::ClientOutdatedInfo {
            stanza: outdated.raw.as_ref().map(stanza_node_of),
        },
    ))
}

pub fn stream_error_notice_of(error: &StreamError) -> pb::ConnectionNotice {
    notice(pb::connection_notice::Notice::StreamError(
        pb::StreamErrorInfo {
            code: error.code.clone(),
            stanza: error.raw.as_ref().map(stanza_node_of),
        },
    ))
}

pub fn stream_replaced_notice_of(_replaced: &StreamReplaced) -> pb::ConnectionNotice {
    notice(pb::connection_notice::Notice::StreamReplaced(pb::Empty {}))
}

pub fn app_state_sync_failed_notice_of(failed: &AppStateSyncFailed) -> pb::ConnectionNotice {
    notice(pb::connection_notice::Notice::AppStateSyncFailed(
        pb::AppStateSyncFailedInfo {
            fatal: failed.fatal.clone(),
            retryable: failed.retryable.clone(),
            skipped: failed.skipped.clone(),
            connected: failed.connected,
        },
    ))
}

/// The server sends seconds; the contract's times are ms (saturating).
pub fn client_expiration_notice_of(changed: &ClientExpirationChanged) -> pb::ConnectionNotice {
    let (primary, secondary, tertiary) = changed.version;
    notice(pb::connection_notice::Notice::ClientExpiration(
        pb::ClientExpirationInfo {
            expires_at: changed.expires_at.map(millis_from_signed_seconds),
            version: Some(pb::ClientBuildVersion {
                primary,
                secondary,
                tertiary,
            }),
            withdrawn: changed.withdrawn,
        },
    ))
}

#[cfg(test)]
#[path = "connection_notice_tests.rs"]
mod tests;
