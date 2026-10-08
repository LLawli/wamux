//! This account's own push name changed (#153, part 2b of #141). The library
//! event reached the socket as RawEvent with the library's JSON.

use wacore::types::events::SelfPushNameUpdated;

use crate::proto::v1 as pb;

/// Both names as sent. The library's `from_server` is left out: both places
/// that build the event set it to true (`client/accessors.rs`,
/// `client/app_state.rs` at 6f07e3a), so it would be a constant on the wire.
pub fn self_push_name_update_of(update: &SelfPushNameUpdated) -> pb::SelfPushNameUpdate {
    pb::SelfPushNameUpdate {
        old_name: update.old_name.clone(),
        new_name: update.new_name.clone(),
    }
}

#[cfg(test)]
#[path = "self_push_name_update_tests.rs"]
mod tests;
