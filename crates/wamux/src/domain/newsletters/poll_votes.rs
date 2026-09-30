//! A channel poll vote, this account's own add-ons, and the live-update
//! subscription (issue #26), relayed over `client.newsletter()`.
//!
//! A channel is not E2E, so its poll vote is not a `pollUpdateMessage` and
//! needs no `message_secret`: it is a plaintext stanza naming each chosen
//! option by `sha256(option_name)`, the same hash the history tallies are
//! keyed on. The library owns the stanza (upstream #1552, #1554, #1555); the
//! core validates the request, relays it, and hands back what came out.

use whatsapp_rust::{Client, Jid, Server};

use super::{millis_from_seconds, require_at_least_one};
use crate::domain::jid_parse::parse_jid;
use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

/// WA Web's own ceiling (`REPEATED_CHILD(<vote>, 0, 1000)` in
/// `WASmaxOutMessagePublishNewsletterPollVoteMixin`), which the library also
/// enforces with a private constant. Checked here so an oversized vote answers
/// InvalidArgument: the library's `InvalidRequest` would reach the caller
/// through `client_err` as Unavailable, which reads as "the core is down".
const MAX_POLL_VOTE_OPTIONS: usize = 1000;

/// Vote in a channel poll. The list is the whole selection: it replaces the
/// previous vote on the server, and an empty one removes it. The answer is the
/// stanza id; the server's verdict arrives as a `ServerAckEvent` with that id.
///
/// No echo is published (unlike the sends in `messaging`, issue #22): a vote
/// puts no message in the channel. `GetMyNewsletterAddOns` reads it back.
pub async fn send_poll_vote(
    client: &Client,
    req: &pb::SendNewsletterPollVoteRequest,
) -> Result<pb::SendNewsletterPollVoteResponse, WamuxError> {
    let jid = require_newsletter_jid(&req.jid)?;
    if req.server_id == 0 {
        return Err(WamuxError::InvalidArgument(
            "server_id must name the poll, got 0".to_string(),
        ));
    }
    let hashes = option_hashes(&req.option_hashes)?;
    let stanza_id = client
        .newsletter()
        .send_poll_vote(&jid, req.server_id, &hashes)
        .await
        .map_err(client_err)?;
    Ok(pb::SendNewsletterPollVoteResponse { stanza_id })
}

/// This account's own reaction and poll vote on a channel's recent messages.
/// Only messages carrying one come back. The tallies cannot answer this: they
/// count every follower.
pub async fn get_my_addons(
    client: &Client,
    req: &pb::GetMyNewsletterAddOnsRequest,
) -> Result<pb::NewsletterMyAddOnsList, WamuxError> {
    let jid = require_newsletter_jid(&req.jid)?;
    require_at_least_one("limit", req.limit)?;
    let addons = client
        .newsletter()
        .get_my_addons(&jid, req.limit)
        .await
        .map_err(client_err)?;
    Ok(pb::NewsletterMyAddOnsList {
        messages: addons.iter().map(my_addons_to_proto).collect(),
    })
}

/// Ask the server to push this channel's live tallies (reactions, poll votes,
/// forwards) as `NewsletterLiveUpdate` events, for the duration it answers.
/// Renewing before that runs out is the caller's timer, not the core's.
pub async fn subscribe_live_updates(
    client: &Client,
    jid: &str,
) -> Result<pb::SubscribeNewsletterLiveUpdatesResponse, WamuxError> {
    let jid = require_newsletter_jid(jid)?;
    let duration_seconds = client
        .newsletter()
        .subscribe_live_updates(jid)
        .await
        .map_err(client_err)?;
    Ok(pb::SubscribeNewsletterLiveUpdatesResponse { duration_seconds })
}

/// A jid on the `newsletter` server. Anything else is refused here rather than
/// by the library, for the same Status reason as `MAX_POLL_VOTE_OPTIONS`.
fn require_newsletter_jid(value: &str) -> Result<Jid, WamuxError> {
    let jid = parse_jid(value)?;
    if jid.server != Server::Newsletter {
        return Err(WamuxError::InvalidArgument(format!(
            "'{value}' is not a channel: expected a jid ending in @newsletter"
        )));
    }
    Ok(jid)
}

/// The wire's `repeated bytes` as the library's fixed-size hashes: each exactly
/// 32 bytes, at most `MAX_POLL_VOTE_OPTIONS`, none repeated.
fn option_hashes(raw: &[Vec<u8>]) -> Result<Vec<[u8; 32]>, WamuxError> {
    if raw.len() > MAX_POLL_VOTE_OPTIONS {
        return Err(WamuxError::InvalidArgument(format!(
            "a poll vote names at most {MAX_POLL_VOTE_OPTIONS} options, got {}",
            raw.len()
        )));
    }
    let mut hashes: Vec<[u8; 32]> = Vec::with_capacity(raw.len());
    for (index, bytes) in raw.iter().enumerate() {
        let hash: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
            WamuxError::InvalidArgument(format!(
                "option_hashes[{index}] must be 32 bytes (sha256 of the option name), got {}",
                bytes.len()
            ))
        })?;
        if hashes.contains(&hash) {
            return Err(WamuxError::InvalidArgument(format!(
                "option_hashes[{index}] repeats an earlier option"
            )));
        }
        hashes.push(hash);
    }
    Ok(hashes)
}

/// One message's add-ons. Presence carries meaning, so absence stays absence:
/// no `poll_vote` means never voted, while a `poll_vote` with no hashes is a
/// vote that was removed (the server keeps it dated).
fn my_addons_to_proto(addons: &whatsapp_rust::NewsletterMyAddOns) -> pb::NewsletterMyAddOns {
    pb::NewsletterMyAddOns {
        server_id: addons.server_id,
        reaction: addons.reaction.as_ref().map(|r| pb::NewsletterMyReaction {
            code: r.code.clone(),
            timestamp: millis_from_seconds(r.timestamp),
        }),
        poll_vote: addons.poll_vote.as_ref().map(|v| pb::NewsletterMyPollVote {
            timestamp: millis_from_seconds(v.timestamp),
            option_hashes: v.option_hashes.iter().map(|h| h.to_vec()).collect(),
        }),
    }
}

#[cfg(test)]
#[path = "poll_votes_tests.rs"]
mod tests;
