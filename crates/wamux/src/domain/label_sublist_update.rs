//! One entry of a label's sublist changed on a linked device (#215).

use wacore::types::events::{LabelSublistChange, LabelSublistUpdate};
use wamux_types::relay_lib_jid;

use crate::proto::v1 as pb;
use crate::proto::v1::label_sublist_update::Change;

/// The mutation as received: its key, the change, and the mutation's own time
/// only when it carried one.
pub fn label_sublist_update_of(update: &LabelSublistUpdate) -> pb::LabelSublistUpdate {
    pb::LabelSublistUpdate {
        predefined_id: update.predefined_id,
        chat: relay_lib_jid(&update.chat_jid),
        action_timestamp: update.action_timestamp.map(|at| at.timestamp_millis()),
        from_full_sync: update.from_full_sync,
        change: change_of(update.change),
    }
}

/// The library's enum is `#[non_exhaustive]`: a change this build does not
/// name is left unset instead of relayed as a wrong case (#149 rule).
fn change_of(change: LabelSublistChange) -> Option<Change> {
    match change {
        LabelSublistChange::Upsert { sub_list_id } => Some(Change::UpsertSubListId(sub_list_id)),
        LabelSublistChange::Remove => Some(Change::Remove(pb::Empty {})),
        _ => None,
    }
}

#[cfg(test)]
#[path = "label_sublist_update_tests.rs"]
mod tests;
