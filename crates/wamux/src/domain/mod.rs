//! Transport-agnostic logic: building bots, mapping events,
//! and the send/media/group/contact helpers used by the thin service layer.

pub mod bot_factory;
pub mod business_profile;
pub mod chat_actions;
pub mod contacts;
pub mod event_mapping;
pub mod group_metadata;
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
pub mod status;
pub(crate) mod sticker_packs;
pub(crate) mod wire_defaults;
pub(crate) mod wire_time;
