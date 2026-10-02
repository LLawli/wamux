//! The two writes: the poll vote, held to the stanza WA Web sent live on
//! 2026-09-25 (#26), and the live-update subscription, answered with the
//! 2026-10-02 capture.

use wacore_binary::{Node, NodeContent};

use crate::captured::{self, CHANNEL, LIVE_SECONDS, VOTED_POLL};
use crate::common::mock_wire::{attr, operation, sent_iq, sent_message};
use crate::harness::{NEWSLETTER, fixture, option_hash};

/// Two options of the poll WA Web voted in when the stanza was measured.
fn good_morning() -> Vec<u8> {
    option_hash("\u{2600}\u{FE0F} GOOD MORNING EVERYONE LET'S GO!!!")
}

fn mondays() -> Vec<u8> {
    option_hash("\u{1F636}\u{200D}\u{1F32B}\u{FE0F} Mondays should be illegal.")
}

/// The `<vote>` payloads under `<votes>`, in order.
fn vote_payloads(message: &Node) -> Vec<Vec<u8>> {
    let votes = message
        .get_optional_child_by_tag(&["votes"])
        .expect("<votes>");
    votes
        .get_children_by_tag("vote")
        .map(|v| match &v.content {
            Some(NodeContent::Bytes(bytes)) => bytes.to_vec(),
            other => panic!("<vote> carries bytes, got {other:?}"),
        })
        .collect()
}

/// The tags directly under `node`, in order.
fn child_tags(node: &Node) -> Vec<&str> {
    node.children()
        .map(|children| children.iter().map(|c| c.tag.as_ref()).collect())
        .unwrap_or_default()
}

// The measured shape: `<message to type="poll" id server_id>` with
// `<meta polltype="vote"/>` then `<votes>`, one `<vote>` per option, in the
// order the request listed them. The RPC answers the stanza's own id.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_poll_vote_puts_the_measured_stanza_on_the_wire() {
    let mut f = fixture("send_poll_vote_puts_the_measured_stanza_on_the_wire").await;
    let request = f.vote(CHANNEL, VOTED_POLL, vec![good_morning(), mondays()]);
    let answer = f
        .channels
        .send_newsletter_poll_vote(request)
        .await
        .expect("vote")
        .into_inner();
    assert!(!answer.stanza_id.is_empty());
    let message = sent_message(&f.mock, &answer.stanza_id).await;
    assert_eq!(attr(&message, "to").as_deref(), Some(CHANNEL));
    assert_eq!(attr(&message, "type").as_deref(), Some("poll"));
    assert_eq!(attr(&message, "server_id").as_deref(), Some("777"));
    assert_eq!(child_tags(&message), vec!["meta", "votes"]);
    let meta = message
        .get_optional_child_by_tag(&["meta"])
        .expect("<meta>");
    assert_eq!(attr(meta, "polltype").as_deref(), Some("vote"));
    assert_eq!(attr(meta, "contenttype"), None);
    assert_eq!(vote_payloads(&message), vec![good_morning(), mondays()]);
    f.cleanup().await;
}

// Clearing a vote is the same stanza with an empty `<votes/>`, as measured.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_selection_sends_an_empty_votes() {
    let mut f = fixture("an_empty_selection_sends_an_empty_votes").await;
    let answer = f
        .channels
        .send_newsletter_poll_vote(f.vote(CHANNEL, VOTED_POLL, Vec::new()))
        .await
        .expect("clear the vote")
        .into_inner();
    let message = sent_message(&f.mock, &answer.stanza_id).await;
    assert_eq!(child_tags(&message), vec!["meta", "votes"]);
    assert!(vote_payloads(&message).is_empty());
    f.cleanup().await;
}

// The subscription is addressed to the channel and names nothing inside; the
// answer is the server's duration, for the edge's renewal timer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribe_live_updates_relays_the_captured_duration() {
    let mut f = fixture("subscribe_live_updates_relays_the_captured_duration").await;
    f.mock
        .answer_iq(NEWSLETTER, "set", "live_updates", captured::live_updates());
    let answer = f
        .channels
        .subscribe_newsletter_live_updates(f.jid_request(CHANNEL))
        .await
        .expect("subscribe")
        .into_inner();
    assert_eq!(answer.duration_seconds, LIVE_SECONDS);
    let iq = sent_iq(&f.mock, NEWSLETTER, "set", "live_updates").await;
    assert_eq!(attr(&iq, "to").as_deref(), Some(CHANNEL));
    let live = operation(&iq).expect("<live_updates>");
    assert!(live.attrs.is_empty(), "{live:?}");
    assert!(child_tags(live).is_empty(), "{live:?}");
    f.cleanup().await;
}
