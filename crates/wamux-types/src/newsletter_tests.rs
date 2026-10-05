//! `NewsletterHistoryQuery`, `NewsletterPollVote`, `NewsletterAddOnsQuery`
//! (#26, #116). The checks moved here from `domain/newsletters` with their
//! messages; the library's own checks answer `InvalidRequest`, which
//! `client_err` turns into Unavailable, so a malformed request stops here.

use wamux_proto::v1 as pb;

use crate::{
    Jid, MAX_NEWSLETTER_VOTE_OPTIONS, NewsletterAddOnsQuery, NewsletterHistoryQuery, NewsletterJid,
    NewsletterPollVote, WamuxError,
};

const CHANNEL: &str = "120363144038483540@newsletter";
const GROUP: &str = "120363041234567890@g.us";

fn invalid_argument<T: std::fmt::Debug>(result: Result<T, WamuxError>) -> String {
    match result {
        Err(WamuxError::InvalidArgument(message)) => message,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

/// sha256("🫠 Just this."), the option voted on live in #26 (2026-09-25).
fn just_this() -> Vec<u8> {
    wacore::poll::compute_option_hash("\u{1FAE0} Just this.").to_vec()
}

fn history(jid: &str, count: u32, before: u64) -> pb::GetNewsletterMessagesRequest {
    pb::GetNewsletterMessagesRequest {
        account: None,
        jid: jid.to_string(),
        count,
        before,
    }
}

fn vote(
    jid: &str,
    server_id: u64,
    option_hashes: Vec<Vec<u8>>,
) -> pb::SendNewsletterPollVoteRequest {
    pb::SendNewsletterPollVoteRequest {
        account: None,
        jid: jid.to_string(),
        server_id,
        option_hashes,
    }
}

fn addons(jid: &str, limit: u32) -> pb::GetMyNewsletterAddOnsRequest {
    pb::GetMyNewsletterAddOnsRequest {
        account: None,
        jid: jid.to_string(),
        limit,
    }
}

fn distinct(n: usize) -> Vec<Vec<u8>> {
    (0..n)
        .map(|i| wacore::poll::compute_option_hash(&i.to_string()).to_vec())
        .collect()
}

#[test]
fn a_history_query_converts_every_field() {
    let query = NewsletterHistoryQuery::try_from(history(CHANNEL, 20, 4321)).unwrap();
    assert_eq!(
        query,
        NewsletterHistoryQuery {
            jid: Jid::parse(CHANNEL).unwrap(),
            count: 20,
            before: Some(4321),
        }
    );
}

// 0 is proto3's absence: start at the newest rather than before row zero.
#[test]
fn a_zero_before_is_the_newest_page() {
    let query = NewsletterHistoryQuery::try_from(history(CHANNEL, 5, 0)).unwrap();
    assert_eq!(query.before, None);
}

#[test]
fn a_history_query_refuses_an_empty_or_malformed_jid() {
    assert_eq!(
        invalid_argument(NewsletterHistoryQuery::try_from(history("", 5, 0))),
        "empty jid"
    );
    let message = invalid_argument(NewsletterHistoryQuery::try_from(history("not a jid", 5, 0)));
    assert!(message.starts_with("invalid jid 'not a jid'"), "{message}");
}

// A zero page size asks the server for nothing; it must read as the caller's
// mistake, not as an empty channel.
#[test]
fn a_history_query_refuses_a_zero_count() {
    assert_eq!(
        invalid_argument(NewsletterHistoryQuery::try_from(history(CHANNEL, 0, 0))),
        "count must be at least 1, got 0"
    );
}

#[test]
fn a_history_query_checks_the_jid_before_the_count() {
    assert_eq!(
        invalid_argument(NewsletterHistoryQuery::try_from(history("", 0, 0))),
        "empty jid"
    );
}

// #99 decides whether history refuses a jid that is not a channel. Until it
// does, any jid converts and goes to the server as it always did.
#[test]
fn a_history_query_takes_any_jid_today() {
    let query = NewsletterHistoryQuery::try_from(history(GROUP, 5, 0)).unwrap();
    assert_eq!(query.jid.to_string(), GROUP);
}

#[test]
fn the_live_vote_hash_is_the_one_measured_on_the_wire() {
    let hex: String = just_this().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        hex,
        "107f7671b96fcff7c2524b29a031dfb5c0c6ea19c9cba6e47de723e5c6983c83"
    );
}

#[test]
fn a_newsletter_vote_converts_the_hashes_in_order() {
    let other = wacore::poll::compute_option_hash("other").to_vec();
    let converted =
        NewsletterPollVote::try_from(vote(CHANNEL, 77, vec![just_this(), other.clone()])).unwrap();
    assert_eq!(converted.jid, NewsletterJid::parse(CHANNEL).unwrap());
    assert_eq!(converted.server_id, 77);
    assert_eq!(converted.option_hashes.len(), 2);
    assert_eq!(converted.option_hashes[0].to_vec(), just_this());
    assert_eq!(converted.option_hashes[1].to_vec(), other);
}

// An empty selection is how a vote is removed, not a malformed request.
#[test]
fn an_empty_newsletter_selection_is_valid() {
    let converted = NewsletterPollVote::try_from(vote(CHANNEL, 77, Vec::new())).unwrap();
    assert!(converted.option_hashes.is_empty());
}

#[test]
fn newsletter_vote_refusals_keep_their_messages() {
    for jid in ["5511999000111@s.whatsapp.net", GROUP] {
        assert_eq!(
            invalid_argument(NewsletterPollVote::try_from(vote(jid, 77, Vec::new()))),
            format!("'{jid}' is not a channel: expected a jid ending in @newsletter")
        );
    }
    assert_eq!(
        invalid_argument(NewsletterPollVote::try_from(vote(CHANNEL, 0, Vec::new()))),
        "server_id must name the poll, got 0"
    );
    assert_eq!(
        invalid_argument(NewsletterPollVote::try_from(vote(
            CHANNEL,
            77,
            vec![just_this(), vec![0u8; 31]]
        ))),
        "option_hashes[1] must be 32 bytes (sha256 of the option name), got 31"
    );
    assert_eq!(
        invalid_argument(NewsletterPollVote::try_from(vote(
            CHANNEL,
            77,
            vec![just_this(), just_this()]
        ))),
        "option_hashes[1] repeats an earlier option"
    );
    assert_eq!(
        invalid_argument(NewsletterPollVote::try_from(vote(
            CHANNEL,
            77,
            distinct(MAX_NEWSLETTER_VOTE_OPTIONS + 1)
        ))),
        "a poll vote names at most 1000 options, got 1001"
    );
}

#[test]
fn the_vote_ceiling_itself_passes() {
    let converted =
        NewsletterPollVote::try_from(vote(CHANNEL, 77, distinct(MAX_NEWSLETTER_VOTE_OPTIONS)))
            .unwrap();
    assert_eq!(converted.option_hashes.len(), MAX_NEWSLETTER_VOTE_OPTIONS);
}

#[test]
fn a_newsletter_vote_checks_jid_then_server_id_then_hashes() {
    let bad_hashes = vec![vec![0u8; 3]];
    let message = invalid_argument(NewsletterPollVote::try_from(vote(
        GROUP,
        0,
        bad_hashes.clone(),
    )));
    assert!(message.contains("is not a channel"), "{message}");
    assert_eq!(
        invalid_argument(NewsletterPollVote::try_from(vote(CHANNEL, 0, bad_hashes))),
        "server_id must name the poll, got 0"
    );
}

#[test]
fn an_addons_query_converts() {
    assert_eq!(
        NewsletterAddOnsQuery::try_from(addons(CHANNEL, 10)).unwrap(),
        NewsletterAddOnsQuery {
            jid: NewsletterJid::parse(CHANNEL).unwrap(),
            limit: 10,
        }
    );
}

#[test]
fn addons_query_refusals_keep_their_messages() {
    assert_eq!(
        invalid_argument(NewsletterAddOnsQuery::try_from(addons(GROUP, 10))),
        format!("'{GROUP}' is not a channel: expected a jid ending in @newsletter")
    );
    assert_eq!(
        invalid_argument(NewsletterAddOnsQuery::try_from(addons(CHANNEL, 0))),
        "limit must be at least 1, got 0"
    );
    assert_eq!(
        invalid_argument(NewsletterAddOnsQuery::try_from(addons("", 0))),
        "empty jid"
    );
}
