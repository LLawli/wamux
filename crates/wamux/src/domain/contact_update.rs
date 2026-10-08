//! A contact sync action, projected onto the contract's typed `ContactUpdate`
//! (#133, part 2 of #74). It used to cross as the library's `serde` JSON.

use wacore::types::events::{ContactRemoved, ContactUpdate};
use wamux_types::{relay_jid, relay_lib_jid};
use whatsapp_rust::waproto::whatsapp::sync_action_value::ContactAction;

use crate::proto::v1 as pb;

/// The mutation's times and the `ContactAction` fields, each unset when absent.
pub fn contact_update_of(update: &ContactUpdate) -> pb::ContactUpdate {
    pb::ContactUpdate {
        jid: relay_lib_jid(&update.jid),
        timestamp: update.timestamp.timestamp_millis(),
        action_timestamp: update.action_timestamp.map(|at| at.timestamp_millis()),
        from_full_sync: update.from_full_sync,
        action: Some(contact_action_of(&update.action)),
        removed: false,
    }
}

/// A saved contact deleted on a linked device (#153): the same message with
/// `removed` true and no action, since the syncd `Remove` carries none.
pub fn contact_removed_of(removal: &ContactRemoved) -> pb::ContactUpdate {
    pb::ContactUpdate {
        jid: relay_lib_jid(&removal.jid),
        timestamp: removal.timestamp.timestamp_millis(),
        action_timestamp: removal.action_timestamp.map(|at| at.timestamp_millis()),
        from_full_sync: removal.from_full_sync,
        action: None,
        removed: true,
    }
}

/// `lidJid` / `pnJid` are strings in the library's protobuf: an empty one is
/// no jid, which `relay_jid` already reads as unset (#120).
fn contact_action_of(action: &ContactAction) -> pb::ContactAction {
    pb::ContactAction {
        full_name: action.full_name.clone(),
        first_name: action.first_name.clone(),
        lid: action.lid_jid.clone().and_then(relay_jid),
        save_on_primary_addressbook: action.save_on_primary_addressbook,
        pn: action.pn_jid.clone().and_then(relay_jid),
        username: action.username.clone(),
    }
}

#[cfg(test)]
#[path = "contact_update_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "contact_removed_tests.rs"]
mod removed_tests;
