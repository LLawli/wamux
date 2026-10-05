//! `PollVoteCast`, `PollVotesToTally`, `NewPoll` (#13, #115).

use wamux_proto::v1 as pb;

use crate::{EncryptedVote, Jid, NewPoll, PollVoteCast, PollVotesToTally, WamuxError};

const CREATOR: &str = "5511999999999@s.whatsapp.net";

fn invalid_argument<T: std::fmt::Debug>(result: Result<T, WamuxError>) -> String {
    match result {
        Err(WamuxError::InvalidArgument(message)) => message,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

fn vote_request(creator: &str, poll_id: &str, secret: Vec<u8>) -> pb::SendPollVoteRequest {
    pb::SendPollVoteRequest {
        account: None,
        chat: None,
        poll_id: poll_id.to_string(),
        poll_creator_jid: creator.to_string(),
        message_secret: secret,
        options: vec!["Sim".to_string()],
    }
}

fn ballot(voter_jid: &str, enc_payload: Vec<u8>) -> pb::PollVote {
    pb::PollVote {
        voter_jid: voter_jid.to_string(),
        enc_payload,
        enc_iv: vec![0x01; 12],
    }
}

fn tally_request(options: &[&str], votes: Vec<pb::PollVote>) -> pb::AggregatePollVotesRequest {
    pb::AggregatePollVotesRequest {
        account: None,
        poll_id: "3EB0POLL".to_string(),
        poll_creator_jid: CREATOR.to_string(),
        message_secret: vec![0x11; 32],
        options: options.iter().map(|o| o.to_string()).collect(),
        votes,
    }
}

#[test]
fn a_vote_converts_every_field() {
    let vote = PollVoteCast::try_from(vote_request(CREATOR, "3EB0POLL", vec![0x11; 32])).unwrap();
    assert_eq!(vote.poll_id.as_str(), "3EB0POLL");
    assert_eq!(vote.creator, Jid::parse(CREATOR).unwrap());
    assert_eq!(vote.message_secret, [0x11; 32]);
    assert_eq!(vote.options, vec!["Sim".to_string()]);
}

// Empty options is a retraction, not a malformed vote.
#[test]
fn a_vote_with_no_options_is_a_retraction() {
    let request = pb::SendPollVoteRequest {
        options: Vec::new(),
        ..vote_request(CREATOR, "3EB0POLL", vec![0x11; 32])
    };
    assert!(PollVoteCast::try_from(request).unwrap().options.is_empty());
}

// Each message is the one the domain answered; the secret's carries the
// offending length and the expected one.
#[test]
fn vote_refusals_keep_their_messages() {
    assert_eq!(
        invalid_argument(PollVoteCast::try_from(vote_request(
            "",
            "3EB0POLL",
            vec![7; 32]
        ))),
        "empty jid"
    );
    let message = invalid_argument(PollVoteCast::try_from(vote_request(
        "not a jid",
        "3EB0POLL",
        vec![7; 32],
    )));
    assert!(message.starts_with("invalid jid 'not a jid'"), "{message}");
    assert_eq!(
        invalid_argument(PollVoteCast::try_from(vote_request(
            CREATOR,
            "",
            vec![7; 32]
        ))),
        "empty poll_id"
    );
    assert_eq!(
        invalid_argument(PollVoteCast::try_from(vote_request(
            CREATOR,
            "3EB0POLL",
            vec![7; 16]
        ))),
        "message_secret must be 32 bytes, got 16"
    );
}

// Creator, then poll id, then secret.
#[test]
fn a_vote_checks_in_the_domain_order() {
    assert_eq!(
        invalid_argument(PollVoteCast::try_from(vote_request("", "", Vec::new()))),
        "empty jid"
    );
    assert_eq!(
        invalid_argument(PollVoteCast::try_from(vote_request(
            CREATOR,
            "",
            Vec::new()
        ))),
        "empty poll_id"
    );
}

// Order is the contract (oldest first, last vote wins): nothing reorders, and
// each voter keeps its own ciphertext.
#[test]
fn a_tally_keeps_the_votes_in_order() {
    let tally = PollVotesToTally::try_from(tally_request(
        &["Sim", "Não"],
        vec![
            ballot(CREATOR, vec![0xAA, 0xBB]),
            ballot("222000222000222@lid", vec![0xCC]),
        ],
    ))
    .unwrap();
    assert_eq!(tally.poll_id.as_str(), "3EB0POLL");
    assert_eq!(tally.creator, Jid::parse(CREATOR).unwrap());
    assert_eq!(tally.message_secret, [0x11; 32]);
    assert_eq!(tally.options, vec!["Sim".to_string(), "Não".to_string()]);
    assert_eq!(
        tally.votes,
        vec![
            EncryptedVote {
                voter: Jid::parse(CREATOR).unwrap(),
                enc_payload: vec![0xAA, 0xBB],
                enc_iv: vec![0x01; 12],
            },
            EncryptedVote {
                voter: Jid::parse("222000222000222@lid").unwrap(),
                enc_payload: vec![0xCC],
                enc_iv: vec![0x01; 12],
            },
        ]
    );
}

#[test]
fn tally_refusals_keep_their_messages() {
    let request = |creator: &str, poll_id: &str, secret: Vec<u8>| pb::AggregatePollVotesRequest {
        poll_creator_jid: creator.to_string(),
        poll_id: poll_id.to_string(),
        message_secret: secret,
        ..tally_request(&["Sim"], vec![ballot(CREATOR, vec![1])])
    };
    assert_eq!(
        invalid_argument(PollVotesToTally::try_from(request(
            "",
            "3EB0POLL",
            vec![7; 32]
        ))),
        "empty jid"
    );
    assert_eq!(
        invalid_argument(PollVotesToTally::try_from(request(
            CREATOR,
            "",
            vec![7; 32]
        ))),
        "empty poll_id"
    );
    assert_eq!(
        invalid_argument(PollVotesToTally::try_from(request(
            CREATOR,
            "3EB0POLL",
            vec![7; 31]
        ))),
        "message_secret must be 32 bytes, got 31"
    );
    assert_eq!(
        invalid_argument(PollVotesToTally::try_from(tally_request(
            &[],
            vec![ballot(CREATOR, vec![1])]
        ))),
        "no options to tally against"
    );
}

#[test]
fn one_unparseable_voter_fails_the_whole_tally() {
    let result = PollVotesToTally::try_from(tally_request(
        &["Sim"],
        vec![ballot(CREATOR, vec![1]), ballot("", vec![2])],
    ));
    assert_eq!(invalid_argument(result), "empty jid");
}

// The options are checked before the voters.
#[test]
fn a_tally_checks_options_before_voters() {
    let result = PollVotesToTally::try_from(tally_request(&[], vec![ballot("", vec![1])]));
    assert_eq!(invalid_argument(result), "no options to tally against");
}

#[test]
fn a_new_poll_copies_every_field() {
    let poll = NewPoll::from(pb::SendPollRequest {
        account: None,
        to: None,
        name: "Cor?".to_string(),
        options: vec!["azul".to_string(), "verde".to_string()],
        selectable_count: 1,
    });
    assert_eq!(
        poll,
        NewPoll {
            name: "Cor?".to_string(),
            options: vec!["azul".to_string(), "verde".to_string()],
            selectable_count: 1,
        }
    );
}
