//! #151 (part 4 of #141): `<notification type="mex">`, routed through
//! `map_event`, so each test also proves the event no longer falls into the
//! RawEvent catch-all. The payload is hand-built: no live capture carried one.

use std::str::FromStr;

use wacore::types::events::{Event, MexNotification};
use whatsapp_rust::Jid;

use super::mex_payload_json_of;
use crate::domain::event_mapping::map_event;
use crate::proto::v1 as pb;
use crate::proto::v1::event_envelope::Event as PbEvent;

const SERVER: &str = "s.whatsapp.net";

/// The one MexNotification the event maps to; anything else is a failure.
fn mex_of(event: Event) -> pb::MexNotification {
    match map_event(&event).as_slice() {
        [PbEvent::Mex(notification)] => notification.clone(),
        other => panic!("expected one MexNotification, got {other:?}"),
    }
}

fn payload() -> serde_json::Value {
    serde_json::from_str(
        r#"{"data":{"xwa2_notify":{"id":"1791549600123456789","name":"x","list":[1,2.5,null,true]}}}"#,
    )
    .expect("test json")
}

#[test]
fn mex_relays_every_field() {
    let event = Event::MexNotification(
        MexNotification::builder()
            .op_name("NotificationUserSettings".to_string())
            .from(Jid::from_str(SERVER).expect("test jid"))
            .stanza_id("3EB0MEX".to_string())
            .offline("1791549600".to_string())
            .payload(payload())
            .build(),
    );
    let notification = mex_of(event);
    assert_eq!(notification.op_name, "NotificationUserSettings");
    assert_eq!(
        notification.from,
        Some(pb::Jid {
            value: SERVER.to_string()
        })
    );
    assert_eq!(notification.stanza_id.as_deref(), Some("3EB0MEX"));
    assert_eq!(notification.offline.as_deref(), Some("1791549600"));
    assert!(!notification.payload_json.is_empty());
}

/// The bytes are the server's JSON again: they parse back to the same value,
/// a 19-digit id included (it is a string on the wire, and stays one).
#[test]
fn mex_payload_json_parses_back_to_the_same_value() {
    let bytes = mex_payload_json_of(&payload());
    let back: serde_json::Value = serde_json::from_slice(&bytes).expect("valid json");
    assert_eq!(back, payload());
}

/// Live traffic: no sender, id or offline marker crosses as unset.
#[test]
fn mex_absent_attributes_stay_unset() {
    let event = Event::MexNotification(
        MexNotification::builder()
            .op_name("Op".to_string())
            .payload(serde_json::json!({}))
            .build(),
    );
    let notification = mex_of(event);
    assert_eq!(
        (
            notification.from,
            notification.stanza_id,
            notification.offline
        ),
        (None, None, None)
    );
    assert_eq!(notification.payload_json, b"{}".to_vec());
}
