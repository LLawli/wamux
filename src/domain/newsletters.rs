//! Channel (`@newsletter`) metadata and history, relayed over `client.newsletter()`.
//!
//! Issue #6: a channel's name existed on WhatsApp's side and never crossed the
//! contract. An inbound newsletter message carries an empty `push_name` and a
//! `sender` equal to the newsletter jid, history sync brings no name, and
//! `GroupService` does not cover them — so a consumer could only ever address
//! these rows by jid. Nothing new is implemented against WhatsApp here; the
//! library already asks, the core just relays the answer.

use whatsapp_rust::features::{
    NewsletterError, NewsletterMessage, NewsletterMetadata, NewsletterRole, NewsletterState,
    NewsletterVerification,
};
use whatsapp_rust::{Client, Jid};

use crate::domain::event_mapping::project_content;
use crate::domain::jid_parse::parse_jid;
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
//   - a role it has no variant for is `None`, so it relays as "" (WA Web drops
//     it the same way);
//   - an absent state reads `Active` and an absent verification `Unverified`,
//     so they relay as "active" / "unverified" instead of "".
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

pub async fn get_metadata(client: &Client, jid: &str) -> Result<pb::Newsletter, WamuxError> {
    let jid = parse_jid(jid)?;
    let found = client
        .newsletter()
        .get_metadata(&jid)
        .await
        .map_err(newsletter_err)?;
    Ok(metadata_to_proto(&found))
}

/// `NotFound` is the one library error with a gRPC meaning of its own; every
/// other one goes through `client_err`, which lifts the server's own
/// rejections (IQ or MEX) into `WaServer`.
fn newsletter_err(err: NewsletterError) -> WamuxError {
    match err {
        NewsletterError::NotFound(jid) => {
            WamuxError::AccountNotFound(format!("no newsletter metadata for {jid}"))
        }
        other => client_err(other),
    }
}

/// Project the library's metadata onto the wire shape.
fn metadata_to_proto(meta: &NewsletterMetadata) -> pb::Newsletter {
    pb::Newsletter {
        jid: meta.jid.to_string(),
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
        verification: verification_token(&meta.verification),
        state: state_token(&meta.state),
        role: meta.role.as_ref().map(role_token).unwrap_or_default(),
        creation_time: meta.creation_time.map_or(0, saturating_i64),
    }
}

// Lowercase tokens, the convention `ReceiptEvent.type` and `CallEvent.action`
// follow. The enums are `#[non_exhaustive]`: a variant added upstream relays
// as "" until it is named here, rather than as a guess.

fn state_token(state: &NewsletterState) -> String {
    match state {
        NewsletterState::Active => "active".into(),
        NewsletterState::Suspended => "suspended".into(),
        NewsletterState::Geosuspended => "geosuspended".into(),
        NewsletterState::Other(raw) => raw.to_lowercase(),
        _ => String::new(),
    }
}

fn verification_token(verification: &NewsletterVerification) -> String {
    match verification {
        NewsletterVerification::Verified => "verified".into(),
        NewsletterVerification::Unverified => "unverified".into(),
        NewsletterVerification::Other(raw) => raw.to_lowercase(),
        _ => String::new(),
    }
}

fn role_token(role: &NewsletterRole) -> String {
    match role {
        NewsletterRole::Owner => "owner".into(),
        NewsletterRole::Admin => "admin".into(),
        NewsletterRole::Subscriber => "subscriber".into(),
        NewsletterRole::Guest => "guest".into(),
        _ => String::new(),
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
    req: &pb::GetNewsletterMessagesRequest,
) -> Result<pb::NewsletterMessageList, WamuxError> {
    let jid: Jid = parse_jid(&req.jid)?;
    require_at_least_one("count", req.count)?;
    // 0 is proto3's "absent": start at the newest rather than before row zero.
    let before = (req.before != 0).then_some(req.before);
    let rows = client
        .newsletter()
        .get_messages(jid, req.count, before)
        .await
        .map_err(client_err)?;
    Ok(pb::NewsletterMessageList {
        messages: rows.iter().map(|row| row_to_proto(row, &req.jid)).collect(),
    })
}

/// A page size (`count`, `limit`) is the caller's to choose, and 0 asks the
/// server for nothing. Refused here so it answers InvalidArgument instead of an
/// empty list, which reads as "this channel has no history".
fn require_at_least_one(field: &str, value: u32) -> Result<(), WamuxError> {
    if value == 0 {
        return Err(WamuxError::InvalidArgument(format!(
            "{field} must be at least 1, got 0"
        )));
    }
    Ok(())
}

/// Project one library row onto the wire shape. Every token is the server's
/// own (`*_raw`, `edit`), so an absent `type` relays as absence rather than as
/// the `text` the typed field defaults to (#1558).
fn row_to_proto(row: &NewsletterMessage, chat: &str) -> pb::NewsletterMessage {
    pb::NewsletterMessage {
        message: Some(row_to_inbound(row, chat)),
        server_id: row.server_id,
        r#type: row.message_type_raw.clone().unwrap_or_default(),
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
        poll_type: row.poll_type_raw.clone().unwrap_or_default(),
        edit: row.edit.as_str().to_string(),
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
fn row_to_inbound(row: &NewsletterMessage, chat: &str) -> pb::InboundMessage {
    let mut out = pb::InboundMessage {
        key: Some(pb::MessageKey {
            remote_jid: chat.to_string(),
            id: row.message_id.clone(),
            from_me: row.is_sender,
            participant: String::new(),
        }),
        chat: chat.to_string(),
        timestamp: millis_from_seconds(row.timestamp),
        raw_message: row
            .message
            .as_ref()
            .map(whatsapp_rust::buffa::Message::encode_to_vec)
            .unwrap_or_default(),
        ..Default::default()
    };
    if let Some(message) = &row.message {
        project_content(&mut out, message, chat);
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

mod poll_votes;
pub use poll_votes::{get_my_addons, send_poll_vote, subscribe_live_updates};

#[cfg(test)]
mod metadata_tests;
