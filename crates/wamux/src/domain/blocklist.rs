//! Blocklist over `client.blocking()` (#197): block, unblock, read the list.

use wamux_types::{Jid, relay_lib_jid};
use whatsapp_rust::Client;
use whatsapp_rust::features::{BlockingError, BlocklistEntry};

use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

pub async fn block_contact(client: &Client, jid: &Jid) -> Result<(), WamuxError> {
    require_user_jid(jid)?;
    client
        .blocking()
        .block(jid.as_lib())
        .await
        .map_err(blocking_err)
}

pub async fn unblock_contact(client: &Client, jid: &Jid) -> Result<(), WamuxError> {
    require_user_jid(jid)?;
    client
        .blocking()
        .unblock(jid.as_lib())
        .await
        .map_err(blocking_err)
}

pub async fn get_blocklist(client: &Client) -> Result<Vec<pb::BlocklistEntry>, WamuxError> {
    let entries = client
        .blocking()
        .get_blocklist()
        .await
        .map_err(blocking_err)?;
    Ok(entries.iter().map(blocklist_entry_of).collect())
}

/// Only a phone user (`@s.whatsapp.net`, any device) or a LID has a blocklist
/// entry, the same `is_pn()`/`is_lid()` the library resolves the pair from. Any
/// other kind is refused here, before a lookup that can never succeed.
pub(crate) fn require_user_jid(jid: &Jid) -> Result<(), WamuxError> {
    let lib = jid.as_lib();
    if lib.is_pn() || lib.is_lid() {
        return Ok(());
    }
    Err(WamuxError::InvalidArgument(format!(
        "{jid} is not a user jid: block and unblock take a phone number (@s.whatsapp.net) or a LID (@lid)"
    )))
}

/// After `require_user_jid`, the library's `InvalidJid` can only mean the client
/// holds no LID<->PN pair for the jid: state the edge can fix, not a bad argument.
pub(crate) fn blocking_err(err: BlockingError) -> WamuxError {
    match err {
        BlockingError::InvalidJid(reason) => WamuxError::FailedPrecondition(reason),
        other => client_err(other),
    }
}

/// A timestamp outside `i64` is left unset rather than clamped, so the edge never
/// sees a made-up time.
pub(crate) fn blocklist_entry_of(entry: &BlocklistEntry) -> pb::BlocklistEntry {
    pb::BlocklistEntry {
        jid: relay_lib_jid(&entry.jid),
        timestamp: entry.timestamp.and_then(|t| i64::try_from(t).ok()),
    }
}

#[cfg(test)]
#[path = "blocklist_tests.rs"]
mod tests;
