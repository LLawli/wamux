//! Issue #26 (upstream #1554): `Event::NewsletterLiveUpdate` maps to the typed
//! `NewsletterLiveUpdate`, votes and forwards included, not the Raw catch-all.
//! The values are the live push captured on 2026-09-25 (WhatsApp channel,
//! poll server_id 777, 12:34 UTC).

use super::*;
use wacore::types::events::{
    NewsletterLiveUpdate, NewsletterLiveUpdateMessage, NewsletterLiveUpdatePollVote,
    NewsletterLiveUpdateReaction,
};

use crate::proto::v1::event_envelope::Event as PbEvent;

const CHANNEL: &str = "120363144038483540@newsletter";

fn vote(option: &str, count: u64) -> NewsletterLiveUpdatePollVote {
    NewsletterLiveUpdatePollVote::builder()
        .option_hash(wacore::poll::compute_option_hash(option))
        .count(count)
        .build()
}

/// The poll (votes + forwards + a reaction) and a plain post (no votes) of
/// that push.
fn captured_push() -> Event {
    let poll = NewsletterLiveUpdateMessage::builder()
        .server_id(777)
        .reactions(vec![
            NewsletterLiveUpdateReaction::builder()
                .code("\u{1F621}".to_string())
                .count(61)
                .build(),
        ])
        .votes(vec![
            vote("\u{1FAE0} Just this.", 183_189),
            vote("\u{1F305} A sunrise photo. Unprompted.", 179_386),
        ])
        .forwards_count(9426)
        .build();
    let post = NewsletterLiveUpdateMessage::builder()
        .server_id(778)
        .reactions(Vec::new())
        .build();
    Event::NewsletterLiveUpdate(
        NewsletterLiveUpdate::builder()
            .newsletter_jid(CHANNEL.parse().expect("channel jid"))
            .messages(vec![poll, post])
            .build(),
    )
}

fn mapped(event: &Event) -> pb::NewsletterLiveUpdate {
    match map_event(event).into_iter().next() {
        Some(PbEvent::NewsletterLiveUpdate(u)) => u,
        other => panic!("expected newsletter live update, got {other:?}"),
    }
}

#[test]
fn a_live_push_maps_its_channel_and_every_message() {
    let update = mapped(&captured_push());
    assert_eq!(update.newsletter_jid, CHANNEL);
    let ids: Vec<u64> = update.messages.iter().map(|m| m.server_id).collect();
    assert_eq!(ids, vec![777, 778]);
}

// The point of #1554: the poll's tallies survive, keyed by the same hash the
// history page and SendNewsletterPollVote use.
#[test]
fn a_poll_keeps_its_tallies_by_option_hash() {
    let poll = &mapped(&captured_push()).messages[0];
    assert_eq!(poll.votes.len(), 2);
    assert_eq!(
        poll.votes[0].option_hash,
        wacore::poll::compute_option_hash("\u{1FAE0} Just this.").to_vec()
    );
    assert_eq!(poll.votes[0].count, 183_189);
    assert_eq!(poll.forwards_count, Some(9426));
    assert_eq!(poll.reactions[0].code, "\u{1F621}");
    assert_eq!(poll.reactions[0].count, 61);
}

// A post has no votes, and a missing `<forwards_count>` stays absent rather
// than becoming a count of zero.
#[test]
fn a_post_has_no_votes_and_an_absent_forward_count_stays_absent() {
    let post = &mapped(&captured_push()).messages[1];
    assert!(post.votes.is_empty());
    assert_eq!(post.forwards_count, None);
}
