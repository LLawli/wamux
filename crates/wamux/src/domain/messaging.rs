//! Build and send chat messages. whatsapp-rust has no convenience senders for
//! normal chats, so we construct `wa::Message` and call `send_message`.

use std::sync::Arc;

use wacore::send::RecipientFanout;
use wamux_types::{
    Jid, LinkPreview, MessageTarget, OutgoingContext, OutgoingText, PresenceState, relay_lib_jid,
};
use whatsapp_rust::buffa::{Enumeration, MessageField};
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::waproto::whatsapp::message::extended_text_message::PreviewType;
use whatsapp_rust::{Client, Jid as LibJid, RevokeType, SendResult};

use crate::domain::outgoing_context::outgoing_context;
use crate::domain::wire_defaults::{nonempty_bytes, nonempty_string, nonzero_i32};
use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

// The core does NOT rewrite recipient JIDs. Routing (e.g. sending via `@c.us`
// vs `@s.whatsapp.net` to work around the library's PN->LID upgrade) is the
// edge's responsibility: it passes the exact JID it wants and the core relays
// to it verbatim. Keeping this transport-pure is a deliberate design choice.

/// The routing (`account`, `to`) was consumed by the service; every content
/// field relays through `build_text_message`, so a new field touches only the
/// builder, never this signature or its call sites (code-review 2026-06-11).
pub async fn send_text(
    client: &Client,
    to: Jid,
    text: &OutgoingText,
) -> Result<SendResult, WamuxError> {
    let message = build_text_message(text)?;
    // The built message rides back out on `SendResult::message` (upstream
    // #1406) so the service can echo it (issue #22): WhatsApp never echoes a
    // send back to the device that made it, and the echo must carry the message
    // that actually went, not a reconstruction.
    client
        .send_message(to.into_lib(), message)
        .await
        .map_err(client_err)
}

/// Pure construction of the outgoing text `wa::Message`. Plain `conversation`
/// only when nothing extra rides along; any mention/quote/preview/ephemeral
/// upgrades it to an `ExtendedTextMessage`. Everything is relayed verbatim:
/// the EDGE fetched the preview and chose the expiration (the core does no
/// outbound HTTP and tracks no chat settings).
pub(crate) fn build_text_message(text: &OutgoingText) -> Result<wa::Message, WamuxError> {
    let plain = text.context == OutgoingContext::default() && text.link_preview.is_none();
    if plain {
        return Ok(wa::Message {
            conversation: Some(text.text.clone()),
            ..Default::default()
        });
    }
    Ok(wa::Message {
        extended_text_message: MessageField::some(extended_text(text)?),
        ..Default::default()
    })
}

fn extended_text(text: &OutgoingText) -> Result<wa::message::ExtendedTextMessage, WamuxError> {
    let mut extended = wa::message::ExtendedTextMessage {
        text: Some(text.text.clone()),
        // Unset for a preview-only message: shared builder, see outgoing_context.
        context_info: outgoing_context(&text.context),
        ..Default::default()
    };
    if let Some(preview) = &text.link_preview {
        copy_link_preview(&mut extended, preview)?;
    }
    Ok(extended)
}

/// Relay the edge-supplied preview verbatim onto the extended text. This
/// waproto has no canonical_url: matched_text IS the URL. preview_type 0
/// (NONE) is both the proto3 default and the wa default, so it relays as the
/// absent field, the same lib-natural form a regular link preview uses.
fn copy_link_preview(
    extended: &mut wa::message::ExtendedTextMessage,
    preview: &LinkPreview,
) -> Result<(), WamuxError> {
    extended.matched_text = nonempty_string(&preview.matched_text);
    extended.title = nonempty_string(&preview.title);
    extended.description = nonempty_string(&preview.description);
    extended.jpeg_thumbnail = nonempty_bytes(&preview.jpeg_thumbnail);
    // waproto 0.7 types this as a closed enum, so an out-of-schema number can
    // no longer ride the wire the way prost's `Option<i32>` let it. Reject it
    // instead of silently dropping the field: dropping would change what the
    // edge asked to relay without telling anyone.
    extended.preview_type = match nonzero_i32(preview.preview_type) {
        None => None,
        Some(value) => Some(PreviewType::from_i32(value).ok_or_else(|| {
            WamuxError::InvalidArgument(format!(
                "unknown link_preview.preview_type {value}; expected a wa PreviewType value"
            ))
        })?),
    };
    Ok(())
}

pub async fn send_reaction(
    client: &Client,
    target: &MessageTarget,
    emoji: &str,
) -> Result<SendResult, WamuxError> {
    let to = target.chat.clone().into_lib();
    let key = wa_message_key(target);
    let message = wa::Message {
        reaction_message: MessageField::some(wa::message::ReactionMessage {
            key: MessageField::some(key),
            text: Some(emoji.to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    client.send_message(to, message).await.map_err(client_err)
}

pub async fn edit_message(
    client: &Client,
    target: &MessageTarget,
    new_text: &str,
) -> Result<SendResult, WamuxError> {
    let to = target.chat.clone().into_lib();
    let new = wa::Message {
        conversation: Some(new_text.to_string()),
        ..Default::default()
    };
    // The library builds the edit container itself; since upstream #1406 it
    // hands it back on `SendResult::message`, which is what lets the service
    // echo an edit at all (#38). `message_id` is the edit stanza's own fresh
    // id, not the target's.
    client
        .edit_message(to, target.id.to_string(), new)
        .await
        .map_err(client_err)
}

/// A status cannot be revoked through DeleteMessage (issue #41): the chat
/// revoke is refused by the library for `status@broadcast`, and the status
/// revoke needs recipients this request has no field for. Checked before the
/// account is looked up, because it is about the request, not the account.
pub fn refuse_status_revoke(target: &MessageTarget, for_everyone: bool) -> Result<(), WamuxError> {
    if !for_everyone || target.chat.as_lib() != &LibJid::status_broadcast() {
        return Ok(());
    }
    Err(WamuxError::InvalidArgument(format!(
        "message {} is a status; revoke it with RevokeStatus, which takes the recipients it was posted to",
        target.id
    )))
}

pub async fn delete_message(
    client: &Client,
    target: &MessageTarget,
    for_everyone: bool,
) -> Result<Option<SendResult>, WamuxError> {
    let to = target.chat.clone().into_lib();
    if for_everyone {
        // A revoke is a message the library builds and sends, so it comes back
        // to be echoed like an edit (#38).
        let result = client
            .revoke_message(to, target.id.to_string(), RevokeType::Sender)
            .await
            .map_err(client_err)?;
        return Ok(Some(result));
    }
    // Delete-for-me is an app-state mutation: nothing goes to the chat, so
    // there is no message to echo.
    client
        .chat_actions()
        .delete_message_for_me(&to, None, target.id.as_str(), target.from_me, false, None)
        .await
        .map_err(client_err)?;
    Ok(None)
}

/// Request on-demand message history (PDO HistorySyncOnDemand): the phone returns
/// `count` messages older than the anchor. Returns a session id that correlates
/// with the eventual `HistorySyncEvent.session_id` on the event stream.
pub async fn fetch_message_history(
    client: &Arc<Client>,
    chat: Jid,
    oldest_msg_id: &str,
    oldest_msg_from_me: bool,
    oldest_msg_timestamp_ms: i64,
    count: i32,
) -> Result<String, WamuxError> {
    client
        .fetch_message_history(
            chat.as_lib(),
            oldest_msg_id,
            oldest_msg_from_me,
            oldest_msg_timestamp_ms,
            count,
        )
        .await
        .map_err(client_err)
}

/// Raise the account's presence, or a chat state in `chat` (#127: the state
/// arrives parsed, and the match has no wildcard arm).
pub async fn send_presence(
    client: &Client,
    chat: Jid,
    state: PresenceState,
) -> Result<(), WamuxError> {
    match state {
        PresenceState::Available => client.presence().set_available().await.map_err(client_err),
        PresenceState::Unavailable => client
            .presence()
            .set_unavailable()
            .await
            .map_err(client_err),
        PresenceState::Composing => client
            .chatstate()
            .send_composing(chat.as_lib())
            .await
            .map_err(client_err),
        PresenceState::Recording => client
            .chatstate()
            .send_recording(chat.as_lib())
            .await
            .map_err(client_err),
        PresenceState::Paused => client
            .chatstate()
            .send_paused(chat.as_lib())
            .await
            .map_err(client_err),
    }
}

/// Send a read receipt: the stanza that turns the other person's ticks blue.
///
/// Issue #20: this used to call `chat_actions().mark_chat_as_read()`, whose own
/// doc line says it is "distinct from `readMessages` IQ receipts -- this syncs
/// state across linked devices". So `MarkRead` marked the chat read on the
/// account's OWN devices, ignored `message_ids` and `sender`, and never put a
/// receipt on the wire. It returned `Empty` either way, which is why nothing
/// downstream noticed. That mutation still exists, under its own name, in
/// `chat_actions::mark_chat_read`.
///
/// Empty `message_ids` is not an error: the library returns before building a
/// stanza, so a caller with nothing to acknowledge costs nothing.
pub async fn mark_read(
    client: &Client,
    chat: &Jid,
    sender: Option<&Jid>,
    message_ids: &[String],
) -> Result<(), WamuxError> {
    let ids: Vec<&str> = message_ids.iter().map(String::as_str).collect();
    client
        .mark_as_read(chat.as_lib(), sender.map(Jid::as_lib), &ids)
        .await
        .map_err(client_err)
}

/// The wa key of a target. An absent participant (a DM) relays as the absent
/// field, never `Some("")` (#20).
pub(crate) fn wa_message_key(target: &MessageTarget) -> wa::MessageKey {
    wa::MessageKey {
        remote_jid: Some(target.chat.to_string()),
        id: Some(target.id.to_string()),
        from_me: Some(target.from_me),
        participant: target.participant.as_ref().map(Jid::to_string),
    }
}

/// Takes the fields instead of the whole `SendResult`: 0.7 sealed that
/// struct (`#[non_exhaustive]`, no public constructor), so a projection that
/// consumed it could not be exercised from a test at all.
pub fn send_result_to_proto(
    message_id: String,
    to: &LibJid,
    fanout: Option<RecipientFanout>,
) -> pb::SendResult {
    pb::SendResult {
        key: Some(sent_message_key(message_id, to)),
        server_timestamp: 0,
        recipient_fanout: fanout.map(recipient_fanout_to_proto),
    }
}

/// The library's DM fan-out count, relayed field for field (#47). Device counts
/// are `usize` there and `uint32` on the wire; a count past `u32::MAX` cannot
/// happen for one recipient, and saturating keeps it from wrapping to a small
/// number that would read as a real one.
pub fn recipient_fanout_to_proto(fanout: RecipientFanout) -> pb::RecipientFanout {
    pb::RecipientFanout {
        addressed: u32::try_from(fanout.addressed).unwrap_or(u32::MAX),
        encrypted: u32::try_from(fanout.encrypted).unwrap_or(u32::MAX),
        skipped_primary: fanout.skipped_primary,
        had_unregistered_device: fanout.had_unregistered_device,
    }
}

/// The key of a message this account just sent: the one the RPC answers with
/// and the one its echo carries, so the two can never disagree.
pub fn sent_message_key(message_id: String, to: &LibJid) -> pb::MessageKey {
    pb::MessageKey {
        chat: relay_lib_jid(to),
        id: message_id,
        from_me: true,
        participant: None,
    }
}

#[cfg(test)]
#[path = "messaging_tests.rs"]
mod tests;
