//! Voting in a poll, and tallying votes the edge already holds (issue #13).
//!
//! Poll *creation* stays in `send_rich`: it is a plain rich-content send. These
//! two are the crypto half, and they are here because the edge cannot do them.
//! A vote is an add-on encrypted under the poll's `message_secret`, its stanza
//! id, and the creator/voter pair in the namespace the poll was addressed in.
//! The LID/PN half of that addressing is resolved against the library's own
//! identity store, so an edge holding only the pairs it happened to see in
//! stanzas fails on votes authored across the LID migration -- silently, and
//! indistinguishably from a wrong key.
//!
//! Relay-pure all the same: the edge supplies the secret, the option names and
//! the votes; the core stores nothing and reads nothing back but its own
//! identity store.

use wamux_types::{EncryptedVote, Jid, PollVoteCast, PollVotesToTally};
use whatsapp_rust::features::{PollOptionResult, PollVoteCiphertext};
use whatsapp_rust::{Client, Jid as LibJid, SendResult};

use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

/// Cast this account's vote. Empty `options` retracts it; a second vote
/// replaces the first (the library sends a PollUpdateMessage either way).
pub async fn send_vote(
    client: &Client,
    chat: Jid,
    vote: &PollVoteCast,
) -> Result<SendResult, WamuxError> {
    client
        .polls()
        .vote(
            chat.into_lib(),
            vote.poll_id.as_str(),
            vote.creator.as_lib(),
            &vote.message_secret,
            &vote.options,
        )
        .await
        .map_err(client_err)
}

/// Tally the votes the request carries. Sends nothing; the only thing read
/// beyond the request is the account's LID/PN store.
pub async fn aggregate_votes(
    client: &Client,
    tally: &PollVotesToTally,
) -> Result<pb::PollTally, WamuxError> {
    let votes = ciphertext_pairs(&tally.votes);
    let undecryptable = count_undecryptable(client, tally, &votes).await;
    let results = client
        .polls()
        .aggregate_votes(
            &tally.options,
            &votes,
            &tally.message_secret,
            tally.poll_id.as_str(),
            tally.creator.as_lib(),
        )
        .await
        .map_err(client_err)?;
    Ok(tally_to_proto(results, undecryptable))
}

/// How many of the supplied votes do not open at all.
///
/// `aggregate_votes` drops an unopenable vote with a `log::warn!` and returns
/// the tally without it, which from the edge is indistinguishable from nobody
/// having voted. The core has no other way to say so, so it asks the same
/// question per vote first. The second pass costs one HKDF/GCM and one
/// namespace lookup per vote -- cheap next to leaving a silent loss uncountable
/// (same reasoning as the two offline-sync counts, issue #11).
async fn count_undecryptable(
    client: &Client,
    tally: &PollVotesToTally,
    votes: &[(&LibJid, PollVoteCiphertext<'_>)],
) -> u32 {
    let mut failed: u32 = 0;
    for (voter, ciphertext) in votes {
        let opened = client
            .polls()
            .decrypt_vote(
                *ciphertext,
                &tally.message_secret,
                tally.poll_id.as_str(),
                tally.creator.as_lib(),
                voter,
            )
            .await;
        failed += u32::from(opened.is_err());
    }
    failed
}

/// Pair each voter with its ciphertext, in the order the votes arrived. The
/// library borrows both halves, so the pairs borrow from the votes.
fn ciphertext_pairs(votes: &[EncryptedVote]) -> Vec<(&LibJid, PollVoteCiphertext<'_>)> {
    votes
        .iter()
        .map(|vote| {
            let ciphertext = PollVoteCiphertext {
                enc_payload: &vote.enc_payload,
                enc_iv: &vote.enc_iv,
            };
            (vote.voter.as_lib(), ciphertext)
        })
        .collect()
}

/// Project the library's per-option results onto the wire shape. Every option
/// the request named comes back, including the ones nobody chose.
fn tally_to_proto(results: Vec<PollOptionResult>, undecryptable: u32) -> pb::PollTally {
    pb::PollTally {
        results: results
            .into_iter()
            .map(|result| pb::PollOptionResult {
                option: result.name,
                voters: result.voters,
            })
            .collect(),
        undecryptable,
    }
}

#[cfg(test)]
#[path = "polls_tests.rs"]
mod tests;
