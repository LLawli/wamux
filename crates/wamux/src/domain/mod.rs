//! Transport-agnostic logic: building bots, mapping events,
//! and the send/media/group/contact helpers used by the thin service layer.

pub mod app_state_update;
pub mod bot_factory;
pub mod business_profile;
pub mod call_event;
pub mod chat_actions;
pub mod contact_update;
pub mod contacts;
pub mod event_mapping;
pub mod group_metadata;
pub mod group_update;
pub mod groups;
pub mod interactive_reply;
pub mod isolate;
pub mod lid_mapping;
pub mod media_transfer;
pub mod messaging;
pub mod newsletters;
pub(crate) mod outgoing_context;
pub mod polls;
pub mod send_rich;
pub(crate) mod stanza_node;
pub mod status;
pub(crate) mod sticker_packs;
pub mod sticker_update;
#[cfg(test)]
pub(crate) mod test_xml;
pub(crate) mod wire_defaults;
pub(crate) mod wire_time;
