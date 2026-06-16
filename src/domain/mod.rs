//! Transport-agnostic logic: building bots, mapping events, parsing jids,
//! and the send/media/group/contact helpers used by the thin service layer.

pub mod bot_factory;
pub mod chat_actions;
pub mod contacts;
pub mod event_mapping;
pub mod groups;
pub mod isolate;
pub mod jid_parse;
pub mod media_transfer;
pub mod messaging;
pub(crate) mod outgoing_context;
pub mod send_rich;
pub mod status;
pub(crate) mod wire_defaults;
