//! The account's reachout timelock changed (#215).

use wacore::types::events::ReachoutTimelockUpdate;
use wamux_types::relay_optional_lib_jid;

use crate::proto::v1 as pb;

/// Every field as the server sent it: nothing absent becomes false or empty.
pub fn reachout_timelock_update_of(update: &ReachoutTimelockUpdate) -> pb::ReachoutTimelockUpdate {
    pb::ReachoutTimelockUpdate {
        enforcement_type: update.state.enforcement_type.clone(),
        is_active: update.state.is_active,
        time_enforcement_ends: update.state.time_enforcement_ends.clone(),
        from: relay_optional_lib_jid(update.from.as_ref()),
        stanza_id: update.stanza_id.clone(),
        offline: update.offline.clone(),
    }
}

#[cfg(test)]
#[path = "reachout_timelock_update_tests.rs"]
mod tests;
