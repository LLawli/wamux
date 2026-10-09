//! Chat-action relays: star/archive/pin/mute/delete a chat or message. Each is
//! a thin wrapper over `client.chat_actions()`; the lib syncs these mutations
//! across the account's linked devices. No policy lives here (the core never
//! invents a mute duration or decides what "archived" means).

use wamux_types::{Jid, MessageTarget};
use whatsapp_rust::Client;

use crate::error::{WamuxError, client_err};

/// Star or unstar a single message. The message-locating fields all come off
/// the `MessageTarget`: a DM has no participant (relayed as `None`, never an
/// empty JID), exactly as `wa_message_key` treats it elsewhere.
pub async fn star_message(
    client: &Client,
    target: &MessageTarget,
    starred: bool,
) -> Result<(), WamuxError> {
    let chat = target.chat.as_lib();
    let participant = target.participant.as_ref().map(Jid::as_lib);
    let id = target.id.as_str();
    let actions = client.chat_actions();
    let result = if starred {
        actions
            .star_message(chat, participant, id, target.from_me)
            .await
    } else {
        actions
            .unstar_message(chat, participant, id, target.from_me)
            .await
    };
    result.map_err(client_err)
}

/// Archive or unarchive a chat. The optional message-range arg is `None`: which
/// messages a sync covers is edge bookkeeping, not the core's concern.
pub async fn archive_chat(client: &Client, chat: Jid, archived: bool) -> Result<(), WamuxError> {
    let actions = client.chat_actions();
    let result = if archived {
        actions.archive_chat(chat.as_lib(), None).await
    } else {
        actions.unarchive_chat(chat.as_lib(), None).await
    };
    result.map_err(client_err)
}

pub async fn pin_chat(client: &Client, chat: Jid, pinned: bool) -> Result<(), WamuxError> {
    let actions = client.chat_actions();
    let result = if pinned {
        actions.pin_chat(chat.as_lib()).await
    } else {
        actions.unpin_chat(chat.as_lib()).await
    };
    result.map_err(client_err)
}

/// Mute (indefinite or until a timestamp) or unmute. `mute_until_ms <= 0` with
/// `muted` means indefinite. A non-future timestamp is the lib's to reject
/// (`AppStateError::InvalidRequest`); `client_err` maps it to InvalidArgument
/// with the lib's reason (#101). The core supplies no clock, decides no duration.
pub async fn mute_chat(
    client: &Client,
    chat: Jid,
    muted: bool,
    mute_until_ms: i64,
) -> Result<(), WamuxError> {
    let actions = client.chat_actions();
    let result = if !muted {
        actions.unmute_chat(chat.as_lib()).await
    } else if mute_until_ms > 0 {
        actions.mute_chat_until(chat.as_lib(), mute_until_ms).await
    } else {
        actions.mute_chat(chat.as_lib()).await
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
        .mark_chat_as_read(chat.as_lib(), true, None)
        .await
        .map_err(client_err)
}

pub async fn mark_unread(client: &Client, chat: &Jid) -> Result<(), WamuxError> {
    client
        .chat_actions()
        .mark_chat_as_read(chat.as_lib(), false, None)
        .await
        .map_err(client_err)
}

/// Delete a whole chat locally. Distinct from `messaging::delete_message`
/// (revoke one message for everyone). `delete_media` relays verbatim; the
/// message-range arg is `None` (edge bookkeeping).
pub async fn delete_chat(client: &Client, chat: Jid, delete_media: bool) -> Result<(), WamuxError> {
    client
        .chat_actions()
        .delete_chat(chat.as_lib(), delete_media, None)
        .await
        .map_err(client_err)
}
