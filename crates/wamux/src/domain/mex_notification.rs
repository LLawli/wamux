//! `<notification type="mex">`, a server GraphQL update (#151, part 4 of
//! #141). It reached the socket as RawEvent with the library's JSON.

use wacore::types::events::MexNotification;
use wamux_types::relay_optional_lib_jid;

use crate::proto::v1 as pb;

pub fn mex_notification_of(notification: &MexNotification) -> pb::MexNotification {
    pb::MexNotification {
        op_name: notification.op_name.clone(),
        from: relay_optional_lib_jid(notification.from.as_ref()),
        stanza_id: notification.stanza_id.clone(),
        offline: notification.offline.clone(),
        payload_json: mex_payload_json_of(&notification.payload),
    }
}

/// The server's JSON body, written back out. The second place allowed to
/// serialize JSON (the wire-JSON check script): the shape is the server's,
/// not the library's, so a library rename cannot move it.
pub(crate) fn mex_payload_json_of(payload: &serde_json::Value) -> Vec<u8> {
    // A `Value` always serializes (string keys, no non-finite numbers), so the
    // default is unreachable; it only avoids a panic on a request path.
    serde_json::to_vec(payload).unwrap_or_default()
}

#[cfg(test)]
#[path = "mex_notification_tests.rs"]
mod tests;
