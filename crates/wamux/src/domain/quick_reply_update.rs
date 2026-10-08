//! Quick replies changed on a linked device (#149, part 2 of #141). The
//! library event reached the socket as RawEvent with the library's JSON.

use wacore::types::events::QuickReplyUpdate;

use crate::proto::v1 as pb;

/// The reply as sent. A deletion is the same mutation with `deleted` true.
pub fn quick_reply_update_of(update: &QuickReplyUpdate) -> pb::QuickReplyUpdate {
    let action = &update.action;
    pb::QuickReplyUpdate {
        id: update.id.clone(),
        timestamp: update.timestamp.timestamp_millis(),
        from_full_sync: update.from_full_sync,
        shortcut: action.shortcut.clone(),
        message: action.message.clone(),
        keywords: action.keywords.clone(),
        count: action.count,
        deleted: action.deleted,
        associated_label_ids: action.associated_label_ids.clone(),
    }
}

#[cfg(test)]
#[path = "quick_reply_update_tests.rs"]
mod tests;
