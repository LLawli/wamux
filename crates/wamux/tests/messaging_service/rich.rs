//! The structured sends: contact, interactive reply, poll and poll vote, and
//! the tally of a vote, which opens it locally and sends nothing.

use tonic::Code;
use wamux::proto::v1 as pb;
use whatsapp_rust::waproto::whatsapp as wa;

use crate::common::mock_wire::sent_message;
use crate::harness::{
    OWN_PN_USER, assert_only_the_probe_was_sent, fixture, jid, key, sender, wire_count,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_contact_reaches_the_peer() {
    let mut f = fixture("send_contact_reaches_the_peer").await;
    let vcard = "BEGIN:VCARD\nVERSION:3.0\nFN:Maria\nTEL:+5511900000004\nEND:VCARD";
    let request = pb::SendContactRequest {
        account: f.a(),
        to: jid(f.peer.pn()),
        display_name: "Maria".into(),
        vcard: vcard.into(),
        quote: None,
    };
    let sent = f
        .messages
        .send_contact(request)
        .await
        .expect("send")
        .into_inner();
    let stanza = sent_message(&f.mock, &sent.key.expect("key").id).await;
    let opened = f.peer.open_dm(&stanza, &sender()).await.expect("open");
    let contact = opened.contact_message.as_option().expect("contactMessage");
    assert_eq!(contact.display_name.as_deref(), Some("Maria"));
    assert_eq!(contact.vcard.as_deref(), Some(vcard));
    f.cleanup().await;
}

// A button reply quotes the offer it answers; the type is DISPLAY_TEXT.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_interactive_reply_reaches_the_peer() {
    let mut f = fixture("send_interactive_reply_reaches_the_peer").await;
    let peer = f.peer.pn().to_string();
    let request = pb::SendInteractiveReplyRequest {
        account: f.a(),
        to: jid(&peer),
        quote: Some(pb::QuoteContext {
            quoted: Some(key(&peer, "3EB0OFFER")),
            participant: peer.clone(),
        }),
        quoted_message: Vec::new(),
        reply: Some(pb::send_interactive_reply_request::Reply::Button(
            pb::ButtonReply {
                selected_id: "opt-1".into(),
                display_text: "Sim".into(),
            },
        )),
    };
    let sent = f
        .messages
        .send_interactive_reply(request)
        .await
        .expect("send")
        .into_inner();
    let stanza = sent_message(&f.mock, &sent.key.expect("key").id).await;
    let opened = f.peer.open_dm(&stanza, &sender()).await.expect("open");
    let reply = opened
        .buttons_response_message
        .as_option()
        .expect("buttonsResponseMessage");
    use wa::message::buttons_response_message::{Response, Type};
    assert_eq!(reply.selected_button_id.as_deref(), Some("opt-1"));
    assert_eq!(
        reply.response,
        Some(Response::SelectedDisplayText("Sim".into()))
    );
    assert_eq!(reply.r#type, Some(Type::DisplayText));
    let context = reply.context_info.as_option().expect("context");
    assert_eq!(context.stanza_id.as_deref(), Some("3EB0OFFER"));
    f.cleanup().await;
}

/// The `poll` RPC as tests use it: single choice between two colors.
fn poll_request(f: &crate::harness::Fixture, selectable_count: u32) -> pb::SendPollRequest {
    pb::SendPollRequest {
        account: f.a(),
        to: jid(f.peer.pn()),
        name: "Qual cor?".into(),
        options: vec!["azul".into(), "verde".into()],
        selectable_count,
    }
}

// The poll carries the secret its votes are encrypted with; the RPC answers
// with that same secret so the edge can open the votes later.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_poll_reaches_the_peer_with_its_secret() {
    let mut f = fixture("send_poll_reaches_the_peer_with_its_secret").await;
    let request = poll_request(&f, 1);
    let sent = f
        .messages
        .send_poll(request)
        .await
        .expect("poll")
        .into_inner();
    assert_eq!(sent.message_secret.len(), 32);
    let stanza = sent_message(&f.mock, &sent.key.expect("key").id).await;
    let opened = f.peer.open_dm(&stanza, &sender()).await.expect("open");
    let poll = opened
        .poll_creation_message_v3
        .as_option()
        .expect("a single-choice poll is v3");
    assert_eq!(poll.name.as_deref(), Some("Qual cor?"));
    let names: Vec<_> = poll.options.iter().map(|o| o.option_name.clone()).collect();
    assert_eq!(names, vec![Some("azul".into()), Some("verde".into())]);
    assert_eq!(poll.selectable_options_count, Some(1));
    let context = opened
        .message_context_info
        .as_option()
        .expect("context info");
    assert_eq!(
        context.message_secret.as_deref(),
        Some(&sent.message_secret[..])
    );
    f.cleanup().await;
}

const POLL_ID: &str = "3EB0POLL";

fn vote_request(f: &crate::harness::Fixture, secret: &[u8]) -> pb::SendPollVoteRequest {
    pb::SendPollVoteRequest {
        account: f.a(),
        chat: jid(f.peer.pn()),
        poll_id: POLL_ID.into(),
        poll_creator_jid: f.peer.pn().to_string(),
        message_secret: secret.to_vec(),
        options: vec!["azul".into()],
    }
}

/// Vote "azul" in the peer's poll and return the `pollUpdateMessage` the peer
/// opened.
async fn cast_vote(
    f: &mut crate::harness::Fixture,
    secret: &[u8],
) -> wa::message::PollUpdateMessage {
    let request = vote_request(f, secret);
    let sent = f
        .messages
        .send_poll_vote(request)
        .await
        .expect("vote")
        .into_inner();
    let stanza = sent_message(&f.mock, &sent.key.expect("key").id).await;
    let opened = f.peer.open_dm(&stanza, &sender()).await.expect("open");
    opened
        .poll_update_message
        .as_option()
        .expect("pollUpdateMessage")
        .clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_poll_vote_reaches_the_peer() {
    let mut f = fixture("send_poll_vote_reaches_the_peer").await;
    let update = cast_vote(&mut f, &[7u8; 32]).await;
    let poll_key = update
        .poll_creation_message_key
        .as_option()
        .expect("poll key");
    assert_eq!(poll_key.id.as_deref(), Some(POLL_ID));
    assert_eq!(poll_key.from_me, Some(false), "the peer created the poll");
    let vote = update.vote.as_option().expect("vote");
    assert!(vote.enc_payload.as_ref().is_some_and(|p| !p.is_empty()));
    assert_eq!(vote.enc_iv.as_ref().map(Vec::len), Some(12));
    f.cleanup().await;
}

// The vote this account sent, opened by the tally with the poll's secret,
// counts for "azul" and nothing else.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aggregate_poll_votes_tallies_the_vote_this_account_sent() {
    let mut f = fixture("aggregate_poll_votes_tallies_the_vote_this_account_sent").await;
    let secret = [7u8; 32];
    let update = cast_vote(&mut f, &secret).await;
    let vote = update.vote.as_option().expect("vote");
    let voter = format!("{OWN_PN_USER}@s.whatsapp.net");
    let request = pb::AggregatePollVotesRequest {
        account: f.a(),
        poll_id: POLL_ID.into(),
        poll_creator_jid: f.peer.pn().to_string(),
        message_secret: secret.to_vec(),
        options: vec!["azul".into(), "verde".into()],
        votes: vec![pb::PollVote {
            voter_jid: voter.clone(),
            enc_payload: vote.enc_payload.clone().unwrap_or_default(),
            enc_iv: vote.enc_iv.clone().unwrap_or_default(),
        }],
    };
    let tally = f
        .messages
        .aggregate_poll_votes(request)
        .await
        .expect("tally")
        .into_inner();
    assert_eq!(tally.undecryptable, 0, "{tally:?}");
    let by_option: Vec<(String, usize)> = tally
        .results
        .iter()
        .map(|r| (r.option.clone(), r.voters.len()))
        .collect();
    assert_eq!(by_option, vec![("azul".into(), 1), ("verde".into(), 0)]);
    f.cleanup().await;
}

/// #101: the library refuses a malformed poll, and the core relays that as
/// Unavailable. `selectable_count` 0 is proto3's default, so leaving it out
/// lands here too. Nothing is sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_invalid_poll_is_unavailable_today() {
    let mut f = fixture("an_invalid_poll_is_unavailable_today").await;
    let before = wire_count(&f);
    let one_option = pb::SendPollRequest {
        options: vec!["azul".into()],
        ..poll_request(&f, 1)
    };
    for (case, request) in [
        ("selectable_count 0", poll_request(&f, 0)),
        ("one option", one_option),
    ] {
        let status = f.messages.send_poll(request).await.expect_err(case);
        assert_eq!(status.code(), Code::Unavailable, "{case}: {status:?}");
    }
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}
