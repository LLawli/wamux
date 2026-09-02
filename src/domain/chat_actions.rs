//! Chat-action relays: star/archive/pin/mute/delete a chat or message. Each is
//! a thin wrapper over `client.chat_actions()`; the lib syncs these mutations
//! across the account's linked devices. No policy lives here (the core never
//! invents a mute duration or decides what "archived" means).

use whatsapp_rust::{Client, Jid};

use crate::domain::jid_parse::parse_jid;
use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

/// Star or unstar a single message. The message-locating fields all come off
/// the wire `MessageKey`: an empty `participant` is a DM (relayed as `None`,
/// never an empty JID), exactly as `proto_key_to_wa` treats it elsewhere.
pub async fn star_message(
    client: &Client,
    target: &pb::MessageKey,
    starred: bool,
) -> Result<(), WamuxError> {
    let chat = parse_jid(&target.remote_jid)?;
    let participant = parse_participant(&target.participant)?;
    let actions = client.chat_actions();
    let result = if starred {
        actions
            .star_message(&chat, participant.as_ref(), &target.id, target.from_me)
            .await
    } else {
        actions
            .unstar_message(&chat, participant.as_ref(), &target.id, target.from_me)
            .await
    };
    result.map_err(client_err)
}

/// Archive or unarchive a chat. The optional message-range arg is `None`: which
/// messages a sync covers is edge bookkeeping, not the core's concern.
pub async fn archive_chat(client: &Client, chat: Jid, archived: bool) -> Result<(), WamuxError> {
    let actions = client.chat_actions();
    let result = if archived {
        actions.archive_chat(&chat, None).await
    } else {
        actions.unarchive_chat(&chat, None).await
    };
    result.map_err(client_err)
}

pub async fn pin_chat(client: &Client, chat: Jid, pinned: bool) -> Result<(), WamuxError> {
    let actions = client.chat_actions();
    let result = if pinned {
        actions.pin_chat(&chat).await
    } else {
        actions.unpin_chat(&chat).await
    };
    result.map_err(client_err)
}

/// Mute (indefinite or until a timestamp) or unmute. `mute_until_ms <= 0` with
/// `muted` means indefinite. A non-future timestamp is the lib's to reject; we
/// relay that error verbatim (the core supplies no clock, decides no duration).
pub async fn mute_chat(
    client: &Client,
    chat: Jid,
    muted: bool,
    mute_until_ms: i64,
) -> Result<(), WamuxError> {
    let actions = client.chat_actions();
    let result = if !muted {
        actions.unmute_chat(&chat).await
    } else if mute_until_ms > 0 {
        actions.mute_chat_until(&chat, mute_until_ms).await
    } else {
        actions.mute_chat(&chat).await
    };
    result.map_err(client_err)
}

/// Mark a chat unread. The symmetric twin of `messaging::mark_read`, kept apart
/// because flipping MarkRead's proto3-default `read` bool would silently change
/// the existing RPC's meaning.
/// Mark the chat read on this account's own linked devices. No receipt leaves
/// for anybody else -- that is `messaging::mark_read`.
///
/// Issue #20: this is the behaviour `MarkRead` used to have under a name that
/// promised the other thing. It is worth keeping: the official app does both,
/// and an edge that wants both composes them. The core does not compose them
/// for it, because bundling would leave no way to ask for one alone.
pub async fn mark_chat_read(client: &Client, chat: &Jid) -> Result<(), WamuxError> {
    client
        .chat_actions()
        .mark_chat_as_read(chat, true, None)
        .await
        .map_err(client_err)
}

pub async fn mark_unread(client: &Client, chat: &Jid) -> Result<(), WamuxError> {
    client
        .chat_actions()
        .mark_chat_as_read(chat, false, None)
        .await
        .map_err(client_err)
}

/// Delete a whole chat locally. Distinct from `messaging::delete_message`
/// (revoke one message for everyone). `delete_media` relays verbatim; the
/// message-range arg is `None` (edge bookkeeping).
pub async fn delete_chat(client: &Client, chat: Jid, delete_media: bool) -> Result<(), WamuxError> {
    client
        .chat_actions()
        .delete_chat(&chat, delete_media, None)
        .await
        .map_err(client_err)
}

/// Parse the optional group participant: proto3's empty string is "no
/// participant" (a DM), which maps to `None`, never a parsed empty JID.
fn parse_participant(value: &str) -> Result<Option<Jid>, WamuxError> {
    if value.is_empty() {
        return Ok(None);
    }
    Ok(Some(parse_jid(value)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Proto3 has no presence on `participant`: an empty string is the DM case
    // and must become None, never Some(<empty jid>).
    #[test]
    fn empty_participant_parses_to_none() {
        assert_eq!(parse_participant("").unwrap(), None);
    }

    #[test]
    fn group_participant_parses_to_some_jid() {
        let participant = parse_participant("5511888888888@s.whatsapp.net")
            .unwrap()
            .expect("non-empty participant must be Some");
        assert_eq!(participant.to_string(), "5511888888888@s.whatsapp.net");
    }

    // A malformed participant is the edge's mistake, surfaced as InvalidArgument
    // (the core never silently drops a bad JID into None).
    #[test]
    fn garbage_participant_is_invalid_argument() {
        assert!(matches!(
            parse_participant("not a jid"),
            Err(WamuxError::InvalidArgument(_))
        ));
    }
}
