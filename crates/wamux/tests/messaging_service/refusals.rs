//! The InvalidArgument branches particular to one RPC, each proving nothing
//! reached the wire, and the order two RPCs check things in (#41, #101).

use tonic::Code;
use wamux::proto::v1 as pb;

use crate::common::mock_wire::account_ref;
use crate::harness::{
    Fixture, assert_only_the_probe_was_sent, fixture, jid, jids, key, text, wire_count,
};

/// Assert every call was InvalidArgument, then that nothing reached the wire.
async fn assert_refused(
    f: &mut Fixture,
    before: crate::harness::WireCount,
    results: Vec<(&str, Result<(), tonic::Status>)>,
) {
    for (case, result) in results {
        let status = result.expect_err(case);
        assert_eq!(status.code(), Code::InvalidArgument, "{case}: {status:?}");
    }
    assert_only_the_probe_was_sent(f, before).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_link_preview_type_is_invalid_argument() {
    let mut f = fixture("an_unknown_link_preview_type_is_invalid_argument").await;
    let before = wire_count(&f);
    let request = pb::SendTextRequest {
        link_preview: Some(pb::LinkPreview {
            matched_text: "https://example.com".into(),
            preview_type: 999,
            ..Default::default()
        }),
        ..text(&f, f.peer.pn(), "https://example.com")
    };
    let result = f.messages.send_text(request).await.map(drop);
    assert_refused(&mut f, before, vec![("preview_type 999", result)]).await;
    f.cleanup().await;
}

fn vote(f: &Fixture, creator: &str, poll_id: &str, secret: Vec<u8>) -> pb::SendPollVoteRequest {
    pb::SendPollVoteRequest {
        account: f.a(),
        chat: jid(f.peer.pn()),
        poll_id: poll_id.into(),
        poll_creator: jid(creator),
        message_secret: secret,
        options: vec!["azul".into()],
    }
}

fn tally(f: &Fixture, options: Vec<String>, voter: &str) -> pb::AggregatePollVotesRequest {
    pb::AggregatePollVotesRequest {
        account: f.a(),
        poll_id: "3EB0POLL".into(),
        poll_creator: jid(f.peer.pn()),
        message_secret: vec![7; 32],
        options,
        votes: vec![pb::PollVote {
            voter: jid(voter),
            enc_payload: vec![1],
            enc_iv: vec![2],
        }],
    }
}

// A vote names its poll by creator, id and the poll's 32-byte secret; a tally
// also needs the options to count and a parseable jid for every voter.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn poll_rpcs_reject_each_malformed_poll() {
    let mut f = fixture("poll_rpcs_reject_each_malformed_poll").await;
    let before = wire_count(&f);
    let creator = f.peer.pn().to_string();
    let peer = f.peer.pn().to_string();
    let votes = [
        ("vote: empty creator", vote(&f, "", "3EB0POLL", vec![7; 32])),
        (
            "vote: malformed creator",
            vote(&f, "not a jid", "3EB0POLL", vec![7; 32]),
        ),
        ("vote: empty poll_id", vote(&f, &creator, "", vec![7; 32])),
        (
            "vote: 31-byte secret",
            vote(&f, &creator, "3EB0POLL", vec![7; 31]),
        ),
    ];
    let tallies = [
        ("tally: no options", tally(&f, Vec::new(), &peer)),
        (
            "tally: a malformed voter",
            tally(&f, vec!["azul".into()], "not a jid"),
        ),
        ("tally: an empty voter", tally(&f, vec!["azul".into()], "")),
    ];
    let mut results = Vec::new();
    for (case, request) in votes {
        results.push((case, f.messages.send_poll_vote(request).await.map(drop)));
    }
    for (case, request) in tallies {
        results.push((
            case,
            f.messages.aggregate_poll_votes(request).await.map(drop),
        ));
    }
    assert_refused(&mut f, before, results).await;
    f.cleanup().await;
}

fn reply(
    f: &Fixture,
    quote: Option<pb::QuoteContext>,
    quoted_message: Vec<u8>,
    with_reply: bool,
) -> pb::SendInteractiveReplyRequest {
    let button = pb::ButtonReply {
        selected_id: "opt-1".into(),
        display_text: "Sim".into(),
    };
    pb::SendInteractiveReplyRequest {
        account: f.a(),
        to: jid(f.peer.pn()),
        quote,
        quoted_message,
        reply: with_reply.then_some(pb::send_interactive_reply_request::Reply::Button(button)),
    }
}

// A reply must name the offer it answers; the offer's payload, when given,
// must be a wa.Message; and a reply needs a shape.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_interactive_reply_rejects_each_malformed_shape() {
    let mut f = fixture("send_interactive_reply_rejects_each_malformed_shape").await;
    let before = wire_count(&f);
    let peer = f.peer.pn().to_string();
    let quote = |id: &str| {
        Some(pb::QuoteContext {
            quoted: Some(key(&peer, id)),
            participant: None,
        })
    };
    let cases = [
        ("no quote", reply(&f, None, Vec::new(), true)),
        (
            "a quote with no key",
            reply(&f, Some(pb::QuoteContext::default()), Vec::new(), true),
        ),
        ("a quote with no id", reply(&f, quote(""), Vec::new(), true)),
        (
            "an offer that is not a wa.Message",
            reply(&f, quote("3EB0OFFER"), vec![0xff; 8], true),
        ),
        (
            "no reply shape",
            reply(&f, quote("3EB0OFFER"), Vec::new(), false),
        ),
    ];
    let mut results = Vec::new();
    for (case, request) in cases {
        results.push((
            case,
            f.messages.send_interactive_reply(request).await.map(drop),
        ));
    }
    assert_refused(&mut f, before, results).await;
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn star_message_rejects_a_malformed_participant() {
    let mut f = fixture("star_message_rejects_a_malformed_participant").await;
    let before = wire_count(&f);
    let target = pb::MessageKey {
        participant: jid("not a jid"),
        ..key(crate::harness::GROUP, "3EB0STAR")
    };
    let request = pb::StarMessageRequest {
        account: f.a(),
        target: Some(target),
        starred: true,
    };
    let result = f.messages.star_message(request).await.map(drop);
    assert_refused(&mut f, before, vec![("a malformed participant", result)]).await;
    f.cleanup().await;
}

// An empty sender is absence (a DM's author is the chat); a sender that is
// there must parse.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mark_read_rejects_a_malformed_sender() {
    let mut f = fixture("mark_read_rejects_a_malformed_sender").await;
    let before = wire_count(&f);
    let request = pb::MarkReadRequest {
        account: f.a(),
        chat: jid(crate::harness::GROUP),
        message_ids: vec!["3EB0A".into()],
        sender: jid("not a jid"),
    };
    let result = f.messages.mark_read(request).await.map(drop);
    assert_refused(&mut f, before, vec![("a malformed sender", result)]).await;
    f.cleanup().await;
}

fn delete(
    account: pb::AccountRef,
    target: pb::MessageKey,
    for_everyone: bool,
) -> pb::DeleteMessageRequest {
    pb::DeleteMessageRequest {
        account: Some(account),
        target: Some(target),
        for_everyone,
    }
}

// #41: a status cannot be revoked through DeleteMessage, and that is about the
// request, so it is refused before the account is looked up: even an unknown
// account gets InvalidArgument, naming RevokeStatus.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_message_refuses_a_status_before_resolving_the_account() {
    let mut f = fixture("delete_message_refuses_a_status_before_resolving_the_account").await;
    let unknown = account_ref(&uuid::Uuid::new_v4().to_string());
    let request = delete(unknown, key("status@broadcast", "3EB0STATUS"), true);
    let status = f
        .messages
        .delete_message(request)
        .await
        .expect_err("a status revoke");
    assert_eq!(status.code(), Code::InvalidArgument, "{status:?}");
    assert!(status.message().contains("RevokeStatus"), "{status:?}");
    f.cleanup().await;
}

/// #101: a malformed chat is refused before the account only on a revoke. A
/// delete-for-me resolves the account first, so an unknown account answers
/// NotFound for the same malformed key.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_message_for_me_checks_the_account_before_the_jid_today() {
    let mut f = fixture("delete_message_for_me_checks_the_account_before_the_jid_today").await;
    let unknown = account_ref(&uuid::Uuid::new_v4().to_string());
    let bad = key("not a jid", "3EB0X");
    let revoke = f
        .messages
        .delete_message(delete(unknown.clone(), bad.clone(), true))
        .await;
    assert_eq!(revoke.expect_err("revoke").code(), Code::InvalidArgument);
    let for_me = f.messages.delete_message(delete(unknown, bad, false)).await;
    assert_eq!(for_me.expect_err("delete for me").code(), Code::NotFound);
    f.cleanup().await;
}

// RevokeStatus validates its shape before the account (#41): an empty id, no
// recipients, or a recipient that is not a jid, even for an unknown account.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoke_status_rejects_each_malformed_shape() {
    let mut f = fixture("revoke_status_rejects_each_malformed_shape").await;
    let before = wire_count(&f);
    let peer = f.peer.pn().to_string();
    let unknown = account_ref(&uuid::Uuid::new_v4().to_string());
    let revoke =
        |account: &pb::AccountRef, id: &str, recipients: Vec<String>| pb::RevokeStatusRequest {
            account: Some(account.clone()),
            message_id: id.into(),
            recipients: jids(&recipients),
        };
    let mut results = Vec::new();
    for account in [f.account.clone(), unknown] {
        let cases = [
            ("an empty id", revoke(&account, "", vec![peer.clone()])),
            ("no recipients", revoke(&account, "3EB0STATUS", Vec::new())),
            (
                "an empty recipient",
                revoke(&account, "3EB0STATUS", vec![String::new()]),
            ),
            (
                "a malformed recipient",
                revoke(&account, "3EB0STATUS", vec!["not a jid".into()]),
            ),
        ];
        for (case, request) in cases {
            results.push((case, f.messages.revoke_status(request).await.map(drop)));
        }
    }
    assert_refused(&mut f, before, results).await;
    f.cleanup().await;
}
