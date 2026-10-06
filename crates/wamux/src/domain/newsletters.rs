//! Channel (`@newsletter`) metadata and history, relayed over `client.newsletter()`.
//!
//! Issue #6: a channel's name existed on WhatsApp's side and never crossed the
//! contract. An inbound newsletter message carries an empty `push_name` and a
//! `sender` equal to the newsletter jid, history sync brings no name, and
//! `GroupService` does not cover them — so a consumer could only ever address
//! these rows by jid. Nothing new is implemented against WhatsApp here; the
//! library already asks, the core just relays the answer.

use wamux_types::newsletter_enums::{edit_attribute_of, poll_type_of};
use wamux_types::{Jid, NewsletterHistoryQuery, relay_jid};
use whatsapp_rust::Client;
use whatsapp_rust::features::{NewsletterError, NewsletterMessage, NewsletterMetadata};

use channel_enums::{
    newsletter_message_type_of, newsletter_role_of, newsletter_state_of, newsletter_verification_of,
};

use crate::domain::event_mapping::project_content;
use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

// WHY METADATA AND HISTORY GO THROUGH THE LIBRARY NOW (issue #56, 2026-09-28).
//
// Both RPCs used to issue their own queries (metadata since #38, history since
// #40), because the library's parse lost what the server sent. Upstream fixed
// each reason: the state/verification case and folding (#1546 → #1557), the
// history `type`/`polltype` tokens (#1548 → #1558), and a missing channel plus
// one id-less list entry (#1560 → #1561). #56 was found while checking the
// last one: the server answers a JID with no channel behind it with
// `{ "id": null, "state": { "type": "NON_EXISTING" } }`, not `null`, and the
// core's own `is_null()` check relayed that as an empty `Newsletter`.
//
// What the library still does differently from the old projection, accepted
// on purpose (the user's call on 2026-09-28):
//
//   - a role it has no variant for is `None`, so it relays as UNSPECIFIED (WA
//     Web drops it the same way);
//   - an absent state reads `Active` and an absent verification `Unverified`,
//     so they relay as ACTIVE / UNVERIFIED instead of UNSPECIFIED. Confirmed
//     again when they became enums (#128, 2026-10-06).
//
// Measured the same day on production, read-only: 9 subscribed channels on
// `pessoal` (none on `trabalho`), through both the list and one get each.
// Every role was SUBSCRIBER, every state ACTIVE, verification always present:
// zero rows affected. If a consumer ever needs those distinctions, the fix is
// upstream (`NewsletterRole::Other`, optional state), not a query of our own.
//
// History: the one remaining difference is an answer without `<messages>`,
// which the library reads as an error where the old path answered an empty
// page. WA Web's parser marks `<messages>` required and throws too
// (`SmaxParsingFailure`), so the library is the one that matches the server's
// contract. A missing channel is an IQ 404, which relays as NotFound.

pub async fn list_subscribed(client: &Client) -> Result<pb::NewsletterList, WamuxError> {
    let found = client
        .newsletter()
        .list_subscribed()
        .await
        .map_err(client_err)?;
    Ok(pb::NewsletterList {
        newsletters: found.iter().map(metadata_to_proto).collect(),
    })
}

pub async fn get_metadata(client: &Client, jid: &Jid) -> Result<pb::Newsletter, WamuxError> {
    let found = client
        .newsletter()
        .get_metadata(jid.as_lib())
        .await
        .map_err(newsletter_err)?;
    Ok(metadata_to_proto(&found))
}

/// `NotFound` is the one library error with a gRPC meaning of its own; every
/// other one goes through `client_err`, which lifts the server's own
/// rejections (IQ or MEX) into `WaServer`.
fn newsletter_err(err: NewsletterError) -> WamuxError {
    match err {
        // Its own variant (#116): `AccountNotFound` would have wrapped this in
        // "account ... not found", naming the wrong thing.
        NewsletterError::NotFound(jid) => {
            WamuxError::NotFound(format!("newsletter {jid} not found"))
        }
        other => client_err(other),
    }
}

/// Project the library's metadata onto the wire shape.
fn metadata_to_proto(meta: &NewsletterMetadata) -> pb::Newsletter {
    let (verification, verification_raw) = newsletter_verification_of(&meta.verification);
    let (state, state_raw) = newsletter_state_of(&meta.state);
    pb::Newsletter {
        jid: relay_jid(meta.jid.to_string()),
        name: meta.name.clone(),
        description: meta.description.clone().unwrap_or_default(),
        subscriber_count: meta.subscriber_count,
        // A direct_path, not a URL: the same shape MediaDescriptor carries, so
        // the edge fetches it the way it fetches any other media path.
        // `picture` is absent on most channels; `preview` is what they carry.
        picture_url: meta
            .picture_url
            .clone()
            .or_else(|| meta.preview_url.clone())
            .unwrap_or_default(),
        verification: verification as i32,
        verification_raw,
        state: state as i32,
        state_raw,
        role: newsletter_role_of(meta.role.as_ref()) as i32,
        creation_time: meta.creation_time.map_or(0, saturating_i64),
    }
}

/// A page of a channel's history, newest first (issue #26).
///
/// Plain IQ, nothing decrypts: a channel is not E2E, so the server hands back
/// the payloads itself. It is also the only place the server's own tallies
/// exist — reactions, poll votes and forwards are counted server-side and ride
/// on no payload — which is the whole reason this RPC is here.
pub async fn get_messages(
    client: &Client,
    query: &NewsletterHistoryQuery,
) -> Result<pb::NewsletterMessageList, WamuxError> {
    let rows = client
        .newsletter()
        .get_messages(query.jid.as_lib().clone(), query.count, query.before)
        .await
        .map_err(client_err)?;
    Ok(pb::NewsletterMessageList {
        messages: rows
            .iter()
            .map(|row| row_to_proto(row, &query.jid))
            .collect(),
    })
}

/// Project one library row onto the wire shape. `type` and `poll_type` are read
/// from the server's own tokens (`*_raw`), so an absent `type` relays as
/// UNSPECIFIED rather than as the TEXT the typed field defaults to (#1558),
/// and a `polltype` on a non-poll row still relays (#128).
fn row_to_proto(row: &NewsletterMessage, chat: &Jid) -> pb::NewsletterMessage {
    let (message_type, type_raw) = newsletter_message_type_of(row.message_type_raw.as_deref());
    let (poll_type, poll_type_raw) = poll_type_of(row.poll_type_raw.as_deref());
    let (edit, edit_raw) = edit_attribute_of(&row.edit);
    pb::NewsletterMessage {
        message: Some(row_to_inbound(row, chat)),
        server_id: row.server_id,
        r#type: message_type as i32,
        type_raw,
        reactions: row
            .reactions
            .iter()
            .map(|reaction| pb::NewsletterReactionCount {
                code: reaction.code.clone(),
                count: reaction.count,
            })
            .collect(),
        votes: row
            .votes
            .iter()
            .map(|vote| pb::NewsletterPollVote {
                option_hash: vote.option_hash.to_vec(),
                count: vote.count,
            })
            .collect(),
        forwards_count: row.forwards_count.unwrap_or(0),
        poll_type: poll_type as i32,
        poll_type_raw,
        edit: edit as i32,
        edit_raw,
        // #51: one node, two units. `original_msg_t` is seconds and
        // `msg_edit_t` is already milliseconds, so only the first converts.
        original_timestamp: row.original_timestamp.map_or(0, millis_from_seconds),
        last_edit_timestamp: row.last_edit_timestamp_ms.map_or(0, saturating_i64),
    }
}

/// The row's message, in the shape the event bus delivers.
///
/// Same projection a live event goes through (`project_content`), because a
/// channel payload IS the same `wa::Message` — it arrives as `<plaintext>`
/// bytes rather than encrypted. One concept, one shape.
///
/// `is_sender` is the only sender information the row carries, so it becomes
/// `key.from_me` and nothing fills `sender`, `push_name` or the alt-namespace
/// jids: a row the server described sparsely does not gain values it never had.
/// `chat` is the jid the caller addressed, not something read off the row.
fn row_to_inbound(row: &NewsletterMessage, chat: &Jid) -> pb::InboundMessage {
    // The jid prints as the caller wrote it for a `@newsletter`; `project_content`
    // (event_mapping, #74) still takes text.
    let chat_text: String = chat.to_string();
    let mut out = pb::InboundMessage {
        key: Some(pb::MessageKey {
            chat: relay_jid(chat_text.clone()),
            id: row.message_id.clone(),
            from_me: row.is_sender,
            participant: None,
        }),
        chat: relay_jid(chat_text.clone()),
        timestamp: millis_from_seconds(row.timestamp),
        raw_message: row
            .message
            .as_ref()
            .map(whatsapp_rust::buffa::Message::encode_to_vec)
            .unwrap_or_default(),
        ..Default::default()
    };
    if let Some(message) = &row.message {
        project_content(&mut out, message, &chat_text);
    }
    out
}

/// The stanza counts in seconds; every timestamp in this contract is
/// milliseconds. Saturating, so a nonsense value cannot come back as a date in
/// the past.
fn millis_from_seconds(seconds: u64) -> i64 {
    saturating_i64(seconds).saturating_mul(1000)
}

/// A wire `u64` into the contract's `int64`, clamped rather than wrapped into
/// a negative time.
fn saturating_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

mod channel_enums;
mod poll_votes;
pub use poll_votes::{get_my_addons, send_poll_vote, subscribe_live_updates};

#[cfg(test)]
mod channel_enums_tests;
#[cfg(test)]
mod metadata_tests;
