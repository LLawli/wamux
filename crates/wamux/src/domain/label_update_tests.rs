//! #149 (part 2 of #141): labels, routed through `map_event`, so each test
//! also proves the event no longer falls into the RawEvent catch-all. Values
//! are the ones the user's phone (pessoal, Business) sent on 2026-10-08 18:47Z
//! to 18:51Z when it created, renamed and deleted a chat list and moved chats
//! in and out of it (read off the production subscription; jids anonymized).
//! The message label is hand-built: the app no longer labels messages.

#![allow(clippy::field_reassign_with_default)] // the generated actions are built by assignment

use std::str::FromStr;

use wacore::types::events::{
    Event, LabelAssociationUpdate, LabelEditUpdate, MessageLabelAssociationUpdate,
};
use whatsapp_rust::Jid;
use whatsapp_rust::waproto::whatsapp::sync_action_value::label_edit_action::ListType;
use whatsapp_rust::waproto::whatsapp::sync_action_value::{
    LabelAssociationAction, LabelEditAction,
};

use crate::domain::event_mapping::map_event;
use crate::proto::v1 as pb;
use crate::proto::v1::event_envelope::Event as PbEvent;
use crate::proto::v1::label_update::Action;

const CHAT: &str = "100000000000149@lid";

/// The instant `ms` after the epoch, as the library's builders take it. A
/// macro because wamux does not depend on chrono and cannot name the type.
macro_rules! at_ms {
    ($ms:expr) => {
        wacore::time::from_millis($ms).expect("test instant")
    };
}

fn lib(value: &str) -> Jid {
    Jid::from_str(value).expect("test jid")
}

fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

/// The one LabelUpdate the event maps to; anything else is a failure.
fn label_of(event: Event) -> pb::LabelUpdate {
    match map_event(&event).as_slice() {
        [PbEvent::Label(update)] => update.clone(),
        other => panic!("expected one LabelUpdate, got {other:?}"),
    }
}

fn edit_event(label_id: &str, action: LabelEditAction, ms: i64) -> Event {
    Event::LabelEditUpdate(
        LabelEditUpdate::builder()
            .label_id(label_id.to_string())
            .timestamp(at_ms!(ms))
            .action(Box::new(action))
            .from_full_sync(false)
            .build(),
    )
}

/// The capture: a new list named "Teste".
fn created_list() -> LabelEditAction {
    let mut action = LabelEditAction::default();
    action.name = Some("Teste".to_string());
    action.color = Some(1);
    action.deleted = Some(false);
    action.order_index = Some(5);
    action.is_active = Some(true);
    action.r#type = Some(ListType::CUSTOM);
    action.is_immutable = Some(false);
    action.mute_end_time_ms = Some(0);
    action
}

#[test]
fn label_edit_relays_the_captured_list() {
    let expected = pb::LabelUpdate {
        label_id: "5".to_string(),
        timestamp: 1_791_485_277_344,
        from_full_sync: false,
        action: Some(Action::Edit(pb::LabelEdit {
            name: Some("Teste".to_string()),
            color: Some(1),
            predefined_id: None,
            deleted: Some(false),
            order_index: Some(5),
            is_active: Some(true),
            list_type: pb::LabelListType::Custom as i32,
            is_immutable: Some(false),
            mute_end_time_ms: Some(0),
        })),
    };
    assert_eq!(
        label_of(edit_event("5", created_list(), 1_791_485_277_344)),
        expected
    );
}

/// The capture: deleting sends the flag with an empty name, colour 0, no
/// order and inactive. It crosses as sent, not as a removal.
#[test]
fn label_edit_of_a_deleted_list_keeps_what_was_sent() {
    let mut action = LabelEditAction::default();
    action.name = Some(String::new());
    action.color = Some(0);
    action.deleted = Some(true);
    action.is_active = Some(false);
    action.r#type = Some(ListType::CUSTOM);
    action.is_immutable = Some(false);
    action.mute_end_time_ms = Some(0);
    let update = label_of(edit_event("5", action, 1_791_485_323_256));
    let Some(Action::Edit(edit)) = update.action else {
        panic!("expected an edit, got {:?}", update.action);
    };
    assert_eq!(
        (edit.name, edit.deleted, edit.order_index, edit.is_active),
        (Some(String::new()), Some(true), None, Some(false))
    );
}

/// The capture's ready-made list carries its predefined id, and each list
/// type crosses as its own case.
#[test]
fn label_edit_maps_the_predefined_id_and_the_list_types() {
    let mut action = created_list();
    action.name = Some("Novo cliente".to_string());
    action.predefined_id = Some(1);
    let update = label_of(edit_event("6", action, 1_791_485_492_767));
    let Some(Action::Edit(edit)) = update.action else {
        panic!("expected an edit, got {:?}", update.action);
    };
    assert_eq!(edit.predefined_id, Some(1));

    let cases = [
        (ListType::NONE, pb::LabelListType::None),
        (ListType::UNREAD, pb::LabelListType::Unread),
        (ListType::PREDEFINED, pb::LabelListType::Predefined),
        (ListType::LEAD, pb::LabelListType::Lead),
        (
            ListType::MENTIONS_AND_REPLIES,
            pb::LabelListType::MentionsAndReplies,
        ),
    ];
    for (lib_type, wire_type) in cases {
        let mut action = created_list();
        action.r#type = Some(lib_type);
        let update = label_of(edit_event("5", action, 1));
        let Some(Action::Edit(edit)) = update.action else {
            panic!("expected an edit, got {:?}", update.action);
        };
        assert_eq!(edit.list_type, wire_type as i32, "for {lib_type:?}");
    }
}

/// The library drops a type it does not know when it decodes, so absent is
/// all the core ever sees: UNSPECIFIED, never UNKNOWN.
#[test]
fn label_edit_without_a_list_type_leaves_it_unspecified() {
    let mut action = created_list();
    action.r#type = None;
    let update = label_of(edit_event("5", action, 1));
    let Some(Action::Edit(edit)) = update.action else {
        panic!("expected an edit, got {:?}", update.action);
    };
    assert_eq!(edit.list_type, pb::LabelListType::Unspecified as i32);
}

fn association(labeled: Option<bool>) -> LabelAssociationAction {
    let mut action = LabelAssociationAction::default();
    action.labeled = labeled;
    action
}

/// The capture: a chat taken out of the list (`labeled` false).
#[test]
fn label_chat_association_relays_the_chat_and_the_flag() {
    let event = Event::LabelAssociationUpdate(
        LabelAssociationUpdate::builder()
            .label_id("5".to_string())
            .chat_jid(lib(CHAT))
            .timestamp(at_ms!(1_791_485_314_420))
            .action(Box::new(association(Some(false))))
            .from_full_sync(false)
            .build(),
    );
    let expected = pb::LabelUpdate {
        label_id: "5".to_string(),
        timestamp: 1_791_485_314_420,
        from_full_sync: false,
        action: Some(Action::Chat(pb::LabelChatAssociation {
            chat: wire(CHAT),
            labeled: Some(false),
            model_meta_data: None,
        })),
    };
    assert_eq!(label_of(event), expected);
}

#[test]
fn label_message_association_relays_the_message() {
    let mut action = association(Some(true));
    action.model_meta_data = Some("meta".to_string());
    let event = Event::MessageLabelAssociationUpdate(
        MessageLabelAssociationUpdate::builder()
            .label_id("7".to_string())
            .chat_jid(lib(CHAT))
            .message_id("3EB0000000000000000149".to_string())
            .timestamp(at_ms!(1_791_485_277_351))
            .action(Box::new(action))
            .from_full_sync(true)
            .build(),
    );
    let expected = pb::LabelUpdate {
        label_id: "7".to_string(),
        timestamp: 1_791_485_277_351,
        from_full_sync: true,
        action: Some(Action::Message(pb::LabelMessageAssociation {
            chat: wire(CHAT),
            message_id: "3EB0000000000000000149".to_string(),
            labeled: Some(true),
            model_meta_data: Some("meta".to_string()),
        })),
    };
    assert_eq!(label_of(event), expected);
}
