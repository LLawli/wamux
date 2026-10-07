//! A channel poll vote, this account's own add-ons, and the live-update
//! subscription (issue #26), relayed over `client.newsletter()`.
//!
//! A channel is not E2E, so its poll vote is not a `pollUpdateMessage` and
//! needs no `message_secret`: it is a plaintext stanza naming each chosen
//! option by `sha256(option_name)`, the same hash the history tallies are
//! keyed on. The library owns the stanza (upstream #1552, #1554, #1555); the
//! core validates the request, relays it, and hands back what came out.

use wamux_types::{NewsletterAddOnsQuery, NewsletterJid, NewsletterPollVote};
use whatsapp_rust::Client;

use crate::domain::wire_time::millis_from_seconds;
use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

/// Vote in a channel poll. The list is the whole selection: it replaces the
/// previous vote on the server, and an empty one removes it. The answer is the
/// stanza id; the server's verdict arrives as a `ServerAckEvent` with that id.
///
/// No echo is published (unlike the sends in `messaging`, issue #22): a vote
/// puts no message in the channel. `GetMyNewsletterAddOns` reads it back.
pub async fn send_poll_vote(
    client: &Client,
    vote: &NewsletterPollVote,
) -> Result<pb::SendNewsletterPollVoteResponse, WamuxError> {
    let stanza_id = client
        .newsletter()
        .send_poll_vote(
            vote.jid.as_jid().as_lib(),
            vote.server_id,
            &vote.option_hashes,
        )
        .await
        .map_err(client_err)?;
    Ok(pb::SendNewsletterPollVoteResponse { stanza_id })
}

/// This account's own reaction and poll vote on a channel's recent messages.
/// Only messages carrying one come back. The tallies cannot answer this: they
/// count every follower.
pub async fn get_my_addons(
    client: &Client,
    query: &NewsletterAddOnsQuery,
) -> Result<pb::NewsletterMyAddOnsList, WamuxError> {
    let addons = client
        .newsletter()
        .get_my_addons(query.jid.as_jid().as_lib(), query.limit)
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
    jid: NewsletterJid,
) -> Result<pb::SubscribeNewsletterLiveUpdatesResponse, WamuxError> {
    let duration_seconds = client
        .newsletter()
        .subscribe_live_updates(jid.as_jid().as_lib().clone())
        .await
        .map_err(client_err)?;
    Ok(pb::SubscribeNewsletterLiveUpdatesResponse { duration_seconds })
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
