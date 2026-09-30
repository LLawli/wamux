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

use whatsapp_rust::features::{PollOptionResult, PollVoteCiphertext};
use whatsapp_rust::{Client, Jid, SendResult};

use crate::domain::jid_parse::parse_jid;
use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

/// WhatsApp generates a 32-byte per-poll secret and the HKDF refuses any other
/// length. Checked here so a mis-sized secret answers InvalidArgument (the
/// caller's mistake) instead of the Unavailable an upstream client error maps
/// to, which would read as "the core is down".
const MESSAGE_SECRET_LEN: usize = 32;

/// Cast this account's vote. Empty `options` retracts it; a second vote
/// replaces the first (the library sends a PollUpdateMessage either way).
pub async fn send_vote(
    client: &Client,
    chat: Jid,
    req: &pb::SendPollVoteRequest,
) -> Result<SendResult, WamuxError> {
    let creator = parse_jid(&req.poll_creator_jid)?;
    require_poll_id(&req.poll_id)?;
    require_message_secret(&req.message_secret)?;
    client
        .polls()
        .vote(
            chat,
            &req.poll_id,
            &creator,
            &req.message_secret,
            &req.options,
        )
        .await
        .map_err(client_err)
}

/// Tally the votes the request carries. Sends nothing; the only thing read
/// beyond the request is the account's LID/PN store.
pub async fn aggregate_votes(
    client: &Client,
    req: &pb::AggregatePollVotesRequest,
) -> Result<pb::PollTally, WamuxError> {
    let creator = parse_jid(&req.poll_creator_jid)?;
    require_poll_id(&req.poll_id)?;
    require_message_secret(&req.message_secret)?;
    require_poll_options(&req.options)?;

    let voters = parse_voters(&req.votes)?;
    let votes = ciphertext_pairs(&voters, &req.votes);
    let undecryptable = count_undecryptable(client, req, &creator, &votes).await;
    let results = client
        .polls()
        .aggregate_votes(
            &req.options,
            &votes,
            &req.message_secret,
            &req.poll_id,
            &creator,
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
    req: &pb::AggregatePollVotesRequest,
    creator: &Jid,
    votes: &[(&Jid, PollVoteCiphertext<'_>)],
) -> u32 {
    let mut failed: u32 = 0;
    for (voter, ciphertext) in votes {
        let opened = client
            .polls()
            .decrypt_vote(
                *ciphertext,
                &req.message_secret,
                &req.poll_id,
                creator,
                voter,
            )
            .await;
        failed += u32::from(opened.is_err());
    }
    failed
}

/// The stanza id keys the vote's HKDF, so an empty one would derive a wrong
/// key rather than fail.
fn require_poll_id(poll_id: &str) -> Result<(), WamuxError> {
    if poll_id.is_empty() {
        return Err(WamuxError::InvalidArgument("empty poll_id".to_string()));
    }
    Ok(())
}

fn require_message_secret(secret: &[u8]) -> Result<(), WamuxError> {
    if secret.len() != MESSAGE_SECRET_LEN {
        return Err(WamuxError::InvalidArgument(format!(
            "message_secret must be {MESSAGE_SECRET_LEN} bytes, got {}",
            secret.len()
        )));
    }
    Ok(())
}

/// A vote names its option by the SHA-256 of the option's name, so with no
/// names to hash every vote would tally against nothing and the answer would
/// be an empty tally rather than an error.
fn require_poll_options(options: &[String]) -> Result<(), WamuxError> {
    if options.is_empty() {
        return Err(WamuxError::InvalidArgument(
            "no options to tally against".to_string(),
        ));
    }
    Ok(())
}

fn parse_voters(votes: &[pb::PollVote]) -> Result<Vec<Jid>, WamuxError> {
    votes
        .iter()
        .map(|vote| parse_jid(&vote.voter_jid))
        .collect()
}

/// Pair each parsed voter with its ciphertext. The voters stay in their own
/// `Vec` because the library borrows them, so they must outlive the pairs.
fn ciphertext_pairs<'a>(
    voters: &'a [Jid],
    votes: &'a [pb::PollVote],
) -> Vec<(&'a Jid, PollVoteCiphertext<'a>)> {
    voters
        .iter()
        .zip(votes)
        .map(|(voter, vote)| {
            let ciphertext = PollVoteCiphertext {
                enc_payload: &vote.enc_payload,
                enc_iv: &vote.enc_iv,
            };
            (voter, ciphertext)
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
mod tests {
    use super::*;

    fn vote(voter_jid: &str) -> pb::PollVote {
        pb::PollVote {
            voter_jid: voter_jid.to_string(),
            enc_payload: vec![0xAA, 0xBB],
            enc_iv: vec![0x01; 12],
        }
    }

    #[test]
    fn a_thirty_two_byte_secret_is_the_only_accepted_length() {
        assert!(require_message_secret(&[0x11; 32]).is_ok());
        let err = require_message_secret(&[0x11; 16]).unwrap_err();
        match err {
            // The error carries the offending value and the expected shape.
            WamuxError::InvalidArgument(msg) => {
                assert!(msg.contains("32"), "got: {msg}");
                assert!(msg.contains("16"), "got: {msg}");
            }
            other => panic!("expected InvalidArgument, got {other:?}"),
        }
    }

    // An empty stanza id derives a wrong key instead of failing, so it must be
    // rejected before it reaches the HKDF.
    #[test]
    fn an_empty_poll_id_is_rejected() {
        assert!(require_poll_id("3EB0POLL").is_ok());
        assert!(matches!(
            require_poll_id(""),
            Err(WamuxError::InvalidArgument(_))
        ));
    }

    // With no option names there is nothing to hash a vote against, and the
    // honest answer is an error, not an empty tally.
    #[test]
    fn tallying_against_no_options_is_rejected() {
        assert!(require_poll_options(&["Sim".to_string()]).is_ok());
        assert!(matches!(
            require_poll_options(&[]),
            Err(WamuxError::InvalidArgument(_))
        ));
    }

    #[test]
    fn voters_parse_in_the_order_the_votes_arrived() {
        let votes = vec![
            vote("5511999999999@s.whatsapp.net"),
            vote("222000222000222@lid"),
        ];
        let voters = parse_voters(&votes).unwrap();
        assert_eq!(voters[0].to_string(), "5511999999999@s.whatsapp.net");
        assert!(voters[1].is_lid());
    }

    // Order is the contract (oldest first, last vote wins), so the pairing must
    // not reorder or drop anything.
    #[test]
    fn pairs_keep_each_voter_with_its_own_ciphertext() {
        let mut votes = vec![
            vote("5511999999999@s.whatsapp.net"),
            vote("5511888888888@s.whatsapp.net"),
        ];
        votes[1].enc_payload = vec![0xCC];
        let voters = parse_voters(&votes).unwrap();
        let pairs = ciphertext_pairs(&voters, &votes);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].0.user, "5511999999999");
        assert_eq!(pairs[0].1.enc_payload, &[0xAA, 0xBB]);
        assert_eq!(pairs[1].0.user, "5511888888888");
        assert_eq!(pairs[1].1.enc_payload, &[0xCC]);
    }

    #[test]
    fn one_unparseable_voter_fails_the_whole_tally() {
        let votes = vec![vote("5511999999999@s.whatsapp.net"), vote("")];
        assert!(matches!(
            parse_voters(&votes),
            Err(WamuxError::InvalidArgument(_))
        ));
    }

    #[test]
    fn an_option_nobody_chose_still_comes_back() {
        let results = vec![
            PollOptionResult {
                name: "Sim".to_string(),
                voters: vec!["5511999999999@s.whatsapp.net".to_string()],
            },
            PollOptionResult {
                name: "Não".to_string(),
                voters: Vec::new(),
            },
        ];
        let tally = tally_to_proto(results, 0);
        assert_eq!(tally.results.len(), 2);
        assert_eq!(tally.results[0].option, "Sim");
        assert_eq!(tally.results[0].voters.len(), 1);
        assert_eq!(tally.results[1].option, "Não");
        assert!(tally.results[1].voters.is_empty());
    }

    // The whole point of the count: a tally with no voters must be
    // distinguishable from a tally whose votes never opened.
    #[test]
    fn undecryptable_votes_are_countable_next_to_an_empty_tally() {
        let empty = tally_to_proto(
            vec![PollOptionResult {
                name: "Sim".to_string(),
                voters: Vec::new(),
            }],
            0,
        );
        assert_eq!(empty.undecryptable, 0);
        let lost = tally_to_proto(
            vec![PollOptionResult {
                name: "Sim".to_string(),
                voters: Vec::new(),
            }],
            3,
        );
        assert_eq!(lost.undecryptable, 3);
    }
}
