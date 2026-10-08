//! #149 (part 2 of #141): quick replies, routed through `map_event`, so each
//! test also proves the event no longer falls into the RawEvent catch-all.
//! Values are the ones the user's phone (pessoal, Business) sent on 2026-10-08
//! 18:50Z when it created, edited and deleted a quick reply (read off the
//! production subscription). Keywords and label ids are hand-built: the
//! capture had none.

#![allow(clippy::field_reassign_with_default)] // the generated actions are built by assignment

use wacore::types::events::{Event, QuickReplyUpdate};
use whatsapp_rust::waproto::whatsapp::sync_action_value::QuickReplyAction;

use crate::domain::event_mapping::map_event;
use crate::proto::v1 as pb;
use crate::proto::v1::event_envelope::Event as PbEvent;

/// The instant `ms` after the epoch, as the library's builders take it. A
/// macro because wamux does not depend on chrono and cannot name the type.
macro_rules! at_ms {
    ($ms:expr) => {
        wacore::time::from_millis($ms).expect("test instant")
    };
}

/// The one QuickReplyUpdate the event maps to; anything else is a failure.
fn quick_reply_of(event: Event) -> pb::QuickReplyUpdate {
    match map_event(&event).as_slice() {
        [PbEvent::QuickReply(update)] => update.clone(),
        other => panic!("expected one QuickReplyUpdate, got {other:?}"),
    }
}

fn reply_event(action: QuickReplyAction, ms: i64) -> Event {
    Event::QuickReplyUpdate(
        QuickReplyUpdate::builder()
            .id("1".to_string())
            .timestamp(at_ms!(ms))
            .action(Box::new(action))
            .from_full_sync(false)
            .build(),
    )
}

#[test]
fn quick_reply_relays_the_captured_reply() {
    let mut action = QuickReplyAction::default();
    action.shortcut = Some("teste".to_string());
    action.message = Some("Esse é apenas um teste".to_string());
    action.keywords = vec!["oi".to_string(), "olá".to_string()];
    action.count = Some(0);
    action.deleted = Some(false);
    action.associated_label_ids = vec!["5".to_string()];
    let expected = pb::QuickReplyUpdate {
        id: "1".to_string(),
        timestamp: 1_791_485_425_414,
        from_full_sync: false,
        shortcut: Some("teste".to_string()),
        message: Some("Esse é apenas um teste".to_string()),
        keywords: vec!["oi".to_string(), "olá".to_string()],
        count: Some(0),
        deleted: Some(false),
        associated_label_ids: vec!["5".to_string()],
    };
    assert_eq!(
        quick_reply_of(reply_event(action, 1_791_485_425_414)),
        expected
    );
}

/// The capture: deleting is the same mutation with the flag and empty texts,
/// not a removal, so it crosses with both.
#[test]
fn quick_reply_deletion_keeps_the_flag() {
    let mut action = QuickReplyAction::default();
    action.shortcut = Some(String::new());
    action.message = Some(String::new());
    action.count = Some(0);
    action.deleted = Some(true);
    let update = quick_reply_of(reply_event(action, 1_791_485_440_126));
    assert_eq!(
        (update.deleted, update.shortcut, update.message),
        (Some(true), Some(String::new()), Some(String::new()))
    );
}
