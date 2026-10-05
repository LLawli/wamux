//! #120: every jid in `common.proto` and `messaging.proto` is a `pb::Jid`.
//! A request's jid is parsed at the boundary: a required one that is unset or
//! empty answers "missing jid" and reaches nothing. A jid the core answers
//! with is relayed verbatim, and an absent one is an unset field.

use tonic::Code;
use wamux::proto::v1 as pb;

use crate::common::mock_wire::sent_message;
use crate::harness::{
    GROUP, PEER_PN_USER, assert_only_the_probe_was_sent, fixture, jid, jids, key, send_to_peer,
    sender, text, wire_count,
};

fn empty() -> Option<pb::Jid> {
    Some(pb::Jid::default())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_quote_and_a_mention_reach_the_peer_from_jid_fields() {
    let mut f = fixture("a_quote_and_a_mention_reach_the_peer_from_jid_fields").await;
    let peer = f.peer.pn().to_string();
    let other = f.other.pn().to_string();
    let request = pb::SendTextRequest {
        mentions: vec![pb::Mention { jid: jid(&other) }],
        quote: Some(pb::QuoteContext {
            quoted: Some(key(&peer, "3EB0QUOTED")),
            participant: jid(&peer),
        }),
        ..text(&f, &peer, "@outro re")
    };
    let sent = f
        .messages
        .send_text(request)
        .await
        .expect("send")
        .into_inner();
    let stanza = sent_message(&f.mock, &sent.key.expect("key").id).await;
    let opened = f.peer.open_dm(&stanza, &sender()).await.expect("open");
    let extended = opened.extended_text_message.as_option().expect("extended");
    let context = extended.context_info.as_option().expect("context");
    assert_eq!(context.mentioned_jid, vec![other]);
    assert_eq!(context.stanza_id.as_deref(), Some("3EB0QUOTED"));
    assert_eq!(context.participant.as_deref(), Some(peer.as_str()));
    f.cleanup().await;
}

// Decided 2026-10-05: unset and empty are the same mistake, and both answer
// the message `to` and `chat` already answered.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unset_or_empty_required_jid_answers_missing_jid() {
    let mut f = fixture("an_unset_or_empty_required_jid_answers_missing_jid").await;
    let before = wire_count(&f);
    let peer = f.peer.pn().to_string();
    let a = f.a();
    let reaction = |chat: Option<pb::Jid>| pb::SendReactionRequest {
        account: a.clone(),
        target: Some(pb::MessageKey {
            chat,
            ..key(&peer, "3EB0TARGET")
        }),
        emoji: "x".into(),
    };
    let unset_chat = reaction(None);
    let empty_chat = reaction(empty());
    let mention = pb::SendTextRequest {
        mentions: vec![pb::Mention { jid: None }],
        ..text(&f, &peer, "@x")
    };
    let vote = pb::SendPollVoteRequest {
        account: a.clone(),
        chat: jid(&peer),
        poll_id: "3EB0POLL".into(),
        poll_creator: None,
        message_secret: vec![7; 32],
        options: vec!["azul".into()],
    };
    let tally = pb::AggregatePollVotesRequest {
        account: a.clone(),
        poll_id: "3EB0POLL".into(),
        poll_creator: jid(&peer),
        message_secret: vec![7; 32],
        options: vec!["azul".into()],
        votes: vec![pb::PollVote {
            voter: empty(),
            enc_payload: vec![1],
            enc_iv: vec![2],
        }],
    };
    let revoke = pb::RevokeStatusRequest {
        account: a.clone(),
        message_id: "3EB0STATUS".into(),
        recipients: vec![pb::Jid::default()],
    };
    let status = pb::PostStatusTextRequest {
        account: a.clone(),
        text: "nunca".into(),
        recipients: vec![pb::Jid::default()],
        ..Default::default()
    };
    let m = &mut f.messages;
    let results = vec![
        (
            "reaction: unset key.chat",
            m.send_reaction(unset_chat).await.map(drop),
        ),
        (
            "reaction: empty key.chat",
            m.send_reaction(empty_chat).await.map(drop),
        ),
        ("text: unset mention", m.send_text(mention).await.map(drop)),
        (
            "vote: unset poll_creator",
            m.send_poll_vote(vote).await.map(drop),
        ),
        (
            "tally: empty voter",
            m.aggregate_poll_votes(tally).await.map(drop),
        ),
        (
            "revoke: empty recipient",
            m.revoke_status(revoke).await.map(drop),
        ),
        (
            "status: empty recipient",
            m.post_status_text(status).await.map(drop),
        ),
    ];
    for (case, result) in results {
        let status = result.expect_err(case);
        assert_eq!(status.code(), Code::InvalidArgument, "{case}: {status:?}");
        assert_eq!(status.message(), "missing jid", "{case}");
    }
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}

// A send names its chat and has no participant: unset, not `Jid { value: "" }`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_send_answers_with_its_chat_and_an_unset_participant() {
    let mut f = fixture("a_send_answers_with_its_chat_and_an_unset_participant").await;
    let dm = send_to_peer(&mut f, "oi").await;
    assert_eq!(dm.chat, jid(f.peer.pn()));
    assert_eq!(dm.participant, None);
    let group = f
        .messages
        .send_text(text(&f, GROUP, "oi grupo"))
        .await
        .expect("group send")
        .into_inner()
        .key
        .expect("key");
    assert_eq!(group.chat, jid(GROUP));
    assert_eq!(group.participant, None);
    f.cleanup().await;
}

// EditMessage answers with the chat as the caller wrote it (as before #38),
// while the library addresses the parsed one: a `@c.us` target comes back
// `@c.us`, verbatim, in a Jid.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn edit_message_echoes_the_chat_as_the_caller_spelled_it() {
    let mut f = fixture("edit_message_echoes_the_chat_as_the_caller_spelled_it").await;
    let sent = send_to_peer(&mut f, "bom dia").await;
    let legacy = format!("{PEER_PN_USER}@c.us");
    let answer = f
        .messages
        .edit_message(pb::EditMessageRequest {
            account: f.a(),
            target: Some(key(&legacy, &sent.id)),
            new_text: "boa tarde".into(),
        })
        .await
        .expect("edit")
        .into_inner();
    let answered = answer.key.expect("key");
    assert_eq!(answered.chat, jid(&legacy));
    assert_eq!(answered.participant, None);
    sent_message(&f.mock, &answered.id).await;
    f.cleanup().await;
}

// Each repeated jid is parsed, so a malformed recipient in the middle of a
// list is refused like a lone one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_malformed_recipient_among_valid_ones_is_refused() {
    let mut f = fixture("a_malformed_recipient_among_valid_ones_is_refused").await;
    let before = wire_count(&f);
    let peer = f.peer.pn().to_string();
    let revoke = pb::RevokeStatusRequest {
        account: f.a(),
        message_id: "3EB0STATUS".into(),
        recipients: jids(&[peer.as_str(), "not a jid", peer.as_str()]),
    };
    let status = f
        .messages
        .revoke_status(revoke)
        .await
        .expect_err("a malformed recipient");
    assert_eq!(status.code(), Code::InvalidArgument, "{status:?}");
    assert!(
        status.message().starts_with("invalid jid 'not a jid'"),
        "{status:?}"
    );
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}
