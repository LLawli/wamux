//! `polls` (#115): the tests that lived inline, moved so the loop can seal
//! them. Checking the secret, the poll id, the options and parsing the voters
//! moved to `wamux-types` (`messaging/polls_tests.rs`) with the same
//! assertions; what stays here is the pairing and the projection.

use wamux_types::{EncryptedVote, Jid};

use crate::proto::v1 as pb;
use whatsapp_rust::features::PollOptionResult;

use super::{ciphertext_pairs, tally_to_proto};

fn vote(voter_jid: &str) -> EncryptedVote {
    EncryptedVote {
        voter: Jid::parse(voter_jid).unwrap(),
        enc_payload: vec![0xAA, 0xBB],
        enc_iv: vec![0x01; 12],
    }
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
    let pairs = ciphertext_pairs(&votes);
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0].0.user, "5511999999999");
    assert_eq!(pairs[0].1.enc_payload, &[0xAA, 0xBB]);
    assert_eq!(pairs[1].0.user, "5511888888888");
    assert_eq!(pairs[1].1.enc_payload, &[0xCC]);
}

#[test]
fn a_lid_voter_keeps_its_namespace() {
    let votes = vec![vote("222000222000222@lid")];
    assert!(ciphertext_pairs(&votes)[0].0.is_lid());
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
    // #120: each voter is a Jid carrying the library's spelling verbatim.
    assert_eq!(
        tally.results[0].voters,
        vec![pb::Jid {
            value: "5511999999999@s.whatsapp.net".to_string()
        }]
    );
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
