//! Labels (chat lists, in the consumer app) changed on a linked device (#149,
//! part 2 of #141). The three library events reached the socket as RawEvent
//! with the library's JSON.

use wacore::types::events::{
    LabelAssociationUpdate, LabelEditUpdate, MessageLabelAssociationUpdate,
};
use wamux_types::relay_lib_jid;
use whatsapp_rust::waproto::whatsapp::sync_action_value::LabelEditAction;
use whatsapp_rust::waproto::whatsapp::sync_action_value::label_edit_action::ListType;

use crate::proto::v1 as pb;
use crate::proto::v1::label_update::Action;

/// The whole edit as sent, the list type as the contract's enum (closed in
/// the library, so no UNKNOWN and no code).
pub fn label_edit_update_of(update: &LabelEditUpdate) -> pb::LabelUpdate {
    label_update_of(
        &update.label_id,
        update.timestamp.timestamp_millis(),
        update.from_full_sync,
        Action::Edit(label_edit_of(&update.action)),
    )
}

pub fn label_association_update_of(update: &LabelAssociationUpdate) -> pb::LabelUpdate {
    let association = pb::LabelChatAssociation {
        chat: relay_lib_jid(&update.chat_jid),
        labeled: update.action.labeled,
        model_meta_data: update.action.model_meta_data.clone(),
    };
    label_update_of(
        &update.label_id,
        update.timestamp.timestamp_millis(),
        update.from_full_sync,
        Action::Chat(association),
    )
}

pub fn message_label_association_update_of(
    update: &MessageLabelAssociationUpdate,
) -> pb::LabelUpdate {
    let association = pb::LabelMessageAssociation {
        chat: relay_lib_jid(&update.chat_jid),
        message_id: update.message_id.clone(),
        labeled: update.action.labeled,
        model_meta_data: update.action.model_meta_data.clone(),
    };
    label_update_of(
        &update.label_id,
        update.timestamp.timestamp_millis(),
        update.from_full_sync,
        Action::Message(association),
    )
}

fn label_update_of(
    label_id: &str,
    timestamp_ms: i64,
    from_full_sync: bool,
    action: Action,
) -> pb::LabelUpdate {
    pb::LabelUpdate {
        label_id: label_id.to_string(),
        timestamp: timestamp_ms,
        from_full_sync,
        action: Some(action),
    }
}

fn label_edit_of(action: &LabelEditAction) -> pb::LabelEdit {
    pb::LabelEdit {
        name: action.name.clone(),
        color: action.color,
        predefined_id: action.predefined_id,
        deleted: action.deleted,
        order_index: action.order_index,
        is_active: action.is_active,
        list_type: label_list_type_of(action.r#type) as i32,
        is_immutable: action.is_immutable,
        mute_end_time_ms: action.mute_end_time_ms,
    }
}

/// One arm per library variant, no wildcard: a variant added upstream fails
/// the build instead of being relayed as a wrong case (#149). Absent is
/// UNSPECIFIED; UNKNOWN is never emitted because the library drops numbers
/// it does not know while decoding.
fn label_list_type_of(list_type: Option<ListType>) -> pb::LabelListType {
    use pb::LabelListType as Kind;
    let Some(list_type) = list_type else {
        return Kind::Unspecified;
    };
    match list_type {
        ListType::NONE => Kind::None,
        ListType::UNREAD => Kind::Unread,
        ListType::GROUPS => Kind::Groups,
        ListType::FAVORITES => Kind::Favorites,
        ListType::PREDEFINED => Kind::Predefined,
        ListType::CUSTOM => Kind::Custom,
        ListType::COMMUNITY => Kind::Community,
        ListType::SERVER_ASSIGNED => Kind::ServerAssigned,
        ListType::DRAFTED => Kind::Drafted,
        ListType::AI_HANDOFF => Kind::AiHandoff,
        ListType::CHANNELS => Kind::Channels,
        ListType::AI_RESPONDING => Kind::AiResponding,
        ListType::ARCHIVED => Kind::Archived,
        ListType::LOCKED => Kind::Locked,
        ListType::INVITES => Kind::Invites,
        ListType::THIRD_PARTY => Kind::ThirdParty,
        ListType::LEAD => Kind::Lead,
        ListType::MENTIONS_AND_REPLIES => Kind::MentionsAndReplies,
    }
}

#[cfg(test)]
#[path = "label_update_tests.rs"]
mod tests;
