//! #150 (part 3 of #141): the connection's lifecycle past its state, routed
//! through `map_event`, so each test also proves the event no longer falls
//! into the RawEvent catch-all. None of these can be provoked safely on a
//! paired account, so the stanzas are hand-built in the shapes the library's
//! handlers read (`client/node_io.rs` at 6f07e3a).

use wacore::iq::dirty::DirtyType;
use wacore::types::events::{
    AppStateSyncFailed, ClientExpirationChanged, ClientOutdated, ConnectFailure,
    ConnectFailureReason, DirtyState, Event, StreamError, StreamReplaced,
};

use crate::domain::event_mapping::map_event;
use crate::domain::stanza_node::stanza_node_of;
use crate::domain::test_xml::node_of_xml;
use crate::proto::v1 as pb;
use crate::proto::v1::connection_notice::Notice;
use crate::proto::v1::event_envelope::Event as PbEvent;

const FAILURE_503: &str = r#"<failure reason="503" message="service unavailable" location="rva"/>"#;
const FAILURE_405: &str = r#"<failure reason="405" location="frc"/>"#;
const STREAM_429: &str = r#"<stream:error code="429"/>"#;
const STREAM_ACK: &str = r#"<stream:error><ack id="3EB0ABC" class="message"/></stream:error>"#;

/// The one ConnectionNotice the event maps to; anything else is a failure.
fn notice_of(event: Event) -> Notice {
    match map_event(&event).as_slice() {
        [PbEvent::ConnectionNotice(pb::ConnectionNotice { notice: Some(n) })] => n.clone(),
        other => panic!("expected one ConnectionNotice, got {other:?}"),
    }
}

#[test]
fn connect_failure_carries_reason_message_and_stanza() {
    let node = node_of_xml(FAILURE_503);
    let event = Event::ConnectFailure(
        ConnectFailure::builder()
            .reason(ConnectFailureReason::ServiceUnavailable)
            .message("service unavailable".to_string())
            .raw(node.clone())
            .build(),
    );
    let expected = Notice::ConnectFailure(pb::ConnectFailureInfo {
        reason: pb::LogoutReason::ServiceUnavailable as i32,
        reason_code: 0,
        message: Some("service unavailable".to_string()),
        stanza: Some(stanza_node_of(&node)),
    });
    assert_eq!(notice_of(event), expected);
}

/// A code the library does not name crosses as UNKNOWN with the number (#73),
/// the same rule as a logout's reason.
#[test]
fn connect_failure_unknown_reason_keeps_the_code() {
    let event = Event::ConnectFailure(
        ConnectFailure::builder()
            .reason(ConnectFailureReason::Unknown(499))
            .build(),
    );
    match notice_of(event) {
        Notice::ConnectFailure(info) => {
            assert_eq!(info.reason, pb::LogoutReason::Unknown as i32);
            assert_eq!(info.reason_code, 499);
            assert_eq!(info.message, None);
        }
        other => panic!("expected connect_failure, got {other:?}"),
    }
}

#[test]
fn connect_failure_without_stanza_leaves_it_unset() {
    let event = Event::ConnectFailure(
        ConnectFailure::builder()
            .reason(ConnectFailureReason::Generic)
            .build(),
    );
    match notice_of(event) {
        Notice::ConnectFailure(info) => {
            assert_eq!(info.reason, pb::LogoutReason::Generic as i32);
            assert_eq!(info.stanza, None);
        }
        other => panic!("expected connect_failure, got {other:?}"),
    }
}

#[test]
fn client_outdated_carries_the_stanza() {
    let node = node_of_xml(FAILURE_405);
    let event = Event::ClientOutdated(ClientOutdated::builder().raw(node.clone()).build());
    let expected = Notice::ClientOutdated(pb::ClientOutdatedInfo {
        stanza: Some(stanza_node_of(&node)),
    });
    assert_eq!(notice_of(event), expected);
}

#[test]
fn stream_error_carries_code_and_stanza() {
    let node = node_of_xml(STREAM_429);
    let event = Event::StreamError(
        StreamError::builder()
            .code("429".to_string())
            .raw(node.clone())
            .build(),
    );
    let expected = Notice::StreamError(pb::StreamErrorInfo {
        code: "429".to_string(),
        stanza: Some(stanza_node_of(&node)),
    });
    assert_eq!(notice_of(event), expected);
}

/// `<stream:error><ack/>` carries no code: the library passes it on empty, and
/// so does the contract. The `<ack>` is in the stanza.
#[test]
fn stream_error_without_code_keeps_it_empty() {
    let node = node_of_xml(STREAM_ACK);
    let event = Event::StreamError(
        StreamError::builder()
            .code(String::new())
            .raw(node.clone())
            .build(),
    );
    let expected = Notice::StreamError(pb::StreamErrorInfo {
        code: String::new(),
        stanza: Some(stanza_node_of(&node)),
    });
    assert_eq!(notice_of(event), expected);
}

#[test]
fn stream_replaced_is_its_own_case() {
    let event = Event::StreamReplaced(StreamReplaced::builder().build());
    assert_eq!(notice_of(event), Notice::StreamReplaced(pb::Empty {}));
}

#[test]
fn app_state_sync_failed_relays_the_three_lists() {
    let event = Event::AppStateSyncFailed(
        AppStateSyncFailed::builder()
            .fatal(vec!["critical_block".to_string()])
            .retryable(vec!["regular_high".to_string(), "regular".to_string()])
            .skipped(vec!["regular_low".to_string()])
            .connected(true)
            .build(),
    );
    let expected = Notice::AppStateSyncFailed(pb::AppStateSyncFailedInfo {
        fatal: vec!["critical_block".to_string()],
        retryable: vec!["regular_high".to_string(), "regular".to_string()],
        skipped: vec!["regular_low".to_string()],
        connected: true,
    });
    assert_eq!(notice_of(event), expected);
}

/// The deadline is Unix seconds in the library; every instant in the contract
/// is ms (#132).
#[test]
fn client_expiration_relays_the_deadline_in_ms_and_the_version() {
    let event = Event::ClientExpirationChanged(
        ClientExpirationChanged::builder()
            .expires_at(1_793_000_000)
            .version((2, 3000, 1_027_123_456))
            .withdrawn(false)
            .build(),
    );
    let expected = Notice::ClientExpiration(pb::ClientExpirationInfo {
        expires_at: Some(1_793_000_000_000),
        version: Some(pb::ClientBuildVersion {
            primary: 2,
            secondary: 3000,
            tertiary: 1_027_123_456,
        }),
        withdrawn: false,
    });
    assert_eq!(notice_of(event), expected);
}

#[test]
fn client_expiration_withdrawn_has_no_deadline() {
    let event = Event::ClientExpirationChanged(
        ClientExpirationChanged::builder()
            .version((2, 3000, 1))
            .withdrawn(true)
            .build(),
    );
    match notice_of(event) {
        Notice::ClientExpiration(info) => {
            assert_eq!(info.expires_at, None);
            assert!(info.withdrawn);
        }
        other => panic!("expected client_expiration, got {other:?}"),
    }
}

/// A nonsense deadline saturates instead of overflowing (wire_time rule).
#[test]
fn client_expiration_deadline_saturates() {
    let event = Event::ClientExpirationChanged(
        ClientExpirationChanged::builder()
            .expires_at(i64::MAX)
            .version((2, 3000, 1))
            .withdrawn(false)
            .build(),
    );
    match notice_of(event) {
        Notice::ClientExpiration(info) => assert_eq!(info.expires_at, Some(i64::MAX)),
        other => panic!("expected client_expiration, got {other:?}"),
    }
}

/// `<ib><dirty>` is an informational hook: the library already resyncs, so it
/// is dropped like `Notification` and `RawNode` (#141 triage) instead of
/// reaching the socket as RawEvent.
#[test]
fn dirty_state_is_dropped() {
    let event = Event::DirtyState(
        DirtyState::builder()
            .dirty_type(DirtyType::Groups)
            .timestamp(1_791_489_000)
            .build(),
    );
    assert_eq!(map_event(&event), Vec::new());
}
