//! Status codes: the account resolution every RPC shares, the server's own
//! refusal, and each InvalidArgument branch the core has (#99 added the last
//! one: metadata and history refuse a jid that is not a channel).

use tonic::Code;
use wamux::proto::v1 as pb;

use crate::captured::{CHANNEL, VOTED_POLL};
use crate::common;
use crate::common::mock_wire::{account_ref, iqs_in, sent_iq};
use crate::harness::{Fixture, MEX, NEWSLETTER, call_every_rpc, fixture, option_hash};

/// A jid that parses and is not a channel.
const GROUP: &str = "120363000000000068@g.us";

fn assert_every(results: &[(&str, Result<(), tonic::Status>)], code: Code) {
    assert_eq!(results.len(), 6, "all 6 RPCs");
    for (rpc, result) in results {
        let status = result.as_ref().expect_err(rpc);
        assert_eq!(status.code(), code, "{rpc}: {status:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_rpc_answers_not_found_for_an_unknown_account() {
    let mut f = fixture("every_rpc_answers_not_found_for_an_unknown_account").await;
    let unknown = account_ref(&uuid::Uuid::new_v4().to_string());
    assert_every(
        &call_every_rpc(&mut f.channels, &unknown).await,
        Code::NotFound,
    );
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_rpc_answers_failed_precondition_when_not_connected() {
    let mut f = fixture("every_rpc_answers_failed_precondition_when_not_connected").await;
    let prefix = common::test_prefix("newsletter_service", "every_rpc_not_connected_idle");
    common::sweep_orphans(f.logged.registry.storage(), &prefix).await;
    let idle = f
        .logged
        .registry
        .create_account(Some(&format!("{prefix}idle")))
        .await
        .expect("idle account");
    let idle_ref = account_ref(&idle.uuid.to_string());
    assert_every(
        &call_every_rpc(&mut f.channels, &idle_ref).await,
        Code::FailedPrecondition,
    );
    f.logged
        .registry
        .delete(&idle)
        .await
        .expect("delete the idle account");
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_rpc_answers_invalid_argument_without_an_account_ref() {
    let mut f = fixture("every_rpc_answers_invalid_argument_without_an_account_ref").await;
    let empty = pb::AccountRef { r#ref: None };
    assert_every(
        &call_every_rpc(&mut f.channels, &empty).await,
        Code::InvalidArgument,
    );
    f.cleanup().await;
}

/// The server's refusal reaches the edge as itself: the code maps (403 is
/// PermissionDenied) and the raw code and text ride as trailers. The vote is
/// the exception: it is a `<message>`, not an IQ, so it answers with its
/// stanza id and the server's verdict arrives later as a `ServerAckEvent`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_iq_rpc_relays_a_server_refusal_as_permission_denied() {
    let mut f = fixture("every_iq_rpc_relays_a_server_refusal_as_permission_denied").await;
    f.mock.answer_iq_error(MEX, "*", "*", 403, "forbidden");
    f.mock
        .answer_iq_error(NEWSLETTER, "*", "*", 403, "forbidden");
    let results = call_every_rpc(&mut f.channels, &f.account.clone()).await;
    assert_eq!(results.len(), 6, "all 6 RPCs");
    for (rpc, result) in &results {
        if *rpc == "SendNewsletterPollVote" {
            assert!(result.is_ok(), "{rpc}: {result:?}");
            continue;
        }
        let status = result.as_ref().expect_err(rpc);
        assert_eq!(status.code(), Code::PermissionDenied, "{rpc}: {status:?}");
        let trailer = |key| status.metadata().get(key).and_then(|v| v.to_str().ok());
        assert_eq!(trailer("wa-code"), Some("403"), "{rpc}");
        assert_eq!(trailer("wa-text"), Some("forbidden"), "{rpc}");
    }
    f.cleanup().await;
}

/// The positive signal for "nothing reached the wire" (#67): a valid call made
/// after the refused ones. The websocket is ordered, so once it lands, anything
/// the refused calls had sent would be there too.
async fn assert_only_the_probe_was_sent(f: &mut Fixture, newsletter_iqs: usize, mex_iqs: usize) {
    f.channels
        .subscribe_newsletter_live_updates(f.jid_request(CHANNEL))
        .await
        .expect("the valid probe");
    sent_iq(&f.mock, NEWSLETTER, "set", "live_updates").await;
    assert_eq!(
        iqs_in(&f.mock, NEWSLETTER),
        newsletter_iqs + 1,
        "only the probe"
    );
    assert_eq!(iqs_in(&f.mock, MEX), mex_iqs, "no MEX query");
    let messages = f.mock.client_messages();
    assert!(messages.is_empty(), "no vote stanza, got {messages:?}");
}

/// The five RPCs that name a channel, all with `jid` as the channel.
async fn channel_scoped_statuses(f: &mut Fixture, jid: &str) -> Vec<(&'static str, tonic::Status)> {
    let request = f.jid_request(jid);
    let history = f.history(jid, 5, 0);
    let vote = f.vote(jid, VOTED_POLL, vec![option_hash("azul")]);
    let add_ons = f.add_ons(jid, 20);
    let c = &mut f.channels;
    vec![
        (
            "GetNewsletterMetadata",
            c.get_newsletter_metadata(request.clone())
                .await
                .map(drop)
                .unwrap_err(),
        ),
        (
            "GetNewsletterMessages",
            c.get_newsletter_messages(history)
                .await
                .map(drop)
                .unwrap_err(),
        ),
        (
            "SendNewsletterPollVote",
            c.send_newsletter_poll_vote(vote)
                .await
                .map(drop)
                .unwrap_err(),
        ),
        (
            "GetMyNewsletterAddOns",
            c.get_my_newsletter_add_ons(add_ons)
                .await
                .map(drop)
                .unwrap_err(),
        ),
        (
            "SubscribeNewsletterLiveUpdates",
            c.subscribe_newsletter_live_updates(request)
                .await
                .map(drop)
                .unwrap_err(),
        ),
    ]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_channel_rpc_rejects_an_empty_or_malformed_jid() {
    let mut f = fixture("every_channel_rpc_rejects_an_empty_or_malformed_jid").await;
    let (newsletter_iqs, mex_iqs) = (iqs_in(&f.mock, NEWSLETTER), iqs_in(&f.mock, MEX));
    for bad in ["", "not a jid"] {
        for (rpc, status) in channel_scoped_statuses(&mut f, bad).await {
            assert_eq!(
                status.code(),
                Code::InvalidArgument,
                "{rpc} with jid {bad:?}"
            );
        }
    }
    assert_only_the_probe_was_sent(&mut f, newsletter_iqs, mex_iqs).await;
    f.cleanup().await;
}

// The vote, the add-ons and the subscription only make sense for a channel and
// refuse anything else before the library sees it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_only_rpcs_reject_a_jid_that_is_not_a_channel() {
    let mut f = fixture("channel_only_rpcs_reject_a_jid_that_is_not_a_channel").await;
    let (newsletter_iqs, mex_iqs) = (iqs_in(&f.mock, NEWSLETTER), iqs_in(&f.mock, MEX));
    let vote = f.vote(GROUP, VOTED_POLL, vec![option_hash("azul")]);
    let add_ons = f.add_ons(GROUP, 20);
    let request = f.jid_request(GROUP);
    let c = &mut f.channels;
    let statuses = [
        (
            "SendNewsletterPollVote",
            c.send_newsletter_poll_vote(vote).await.map(drop),
        ),
        (
            "GetMyNewsletterAddOns",
            c.get_my_newsletter_add_ons(add_ons).await.map(drop),
        ),
        (
            "SubscribeNewsletterLiveUpdates",
            c.subscribe_newsletter_live_updates(request).await.map(drop),
        ),
    ];
    for (rpc, result) in statuses {
        let status = result.expect_err(rpc);
        assert_eq!(status.code(), Code::InvalidArgument, "{rpc}: {status:?}");
    }
    assert_only_the_probe_was_sent(&mut f, newsletter_iqs, mex_iqs).await;
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_poll_vote_rejects_each_malformed_selection() {
    let mut f = fixture("send_poll_vote_rejects_each_malformed_selection").await;
    let (newsletter_iqs, mex_iqs) = (iqs_in(&f.mock, NEWSLETTER), iqs_in(&f.mock, MEX));
    let azul = option_hash("azul");
    let too_many: Vec<Vec<u8>> = (0..1001)
        .map(|i| option_hash(&format!("option {i}")))
        .collect();
    let cases = [
        ("no poll", f.vote(CHANNEL, 0, vec![azul.clone()])),
        (
            "a 31-byte hash",
            f.vote(CHANNEL, VOTED_POLL, vec![vec![0u8; 31]]),
        ),
        (
            "a repeated option",
            f.vote(CHANNEL, VOTED_POLL, vec![azul.clone(), azul]),
        ),
        ("1001 options", f.vote(CHANNEL, VOTED_POLL, too_many)),
    ];
    for (case, request) in cases {
        let status = f
            .channels
            .send_newsletter_poll_vote(request)
            .await
            .expect_err(case);
        assert_eq!(status.code(), Code::InvalidArgument, "{case}: {status:?}");
    }
    assert_only_the_probe_was_sent(&mut f, newsletter_iqs, mex_iqs).await;
    f.cleanup().await;
}

// A page size of 0 asks the server for nothing: refused, so it does not read
// as "this channel has no history".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zero_page_sizes_are_invalid_argument() {
    let mut f = fixture("zero_page_sizes_are_invalid_argument").await;
    let (newsletter_iqs, mex_iqs) = (iqs_in(&f.mock, NEWSLETTER), iqs_in(&f.mock, MEX));
    let history = f.history(CHANNEL, 0, 0);
    let add_ons = f.add_ons(CHANNEL, 0);
    let count = f
        .channels
        .get_newsletter_messages(history)
        .await
        .expect_err("count 0");
    let limit = f
        .channels
        .get_my_newsletter_add_ons(add_ons)
        .await
        .expect_err("limit 0");
    assert_eq!(count.code(), Code::InvalidArgument, "{count:?}");
    assert_eq!(limit.code(), Code::InvalidArgument, "{limit:?}");
    assert_only_the_probe_was_sent(&mut f, newsletter_iqs, mex_iqs).await;
    f.cleanup().await;
}

/// #99: metadata and history refuse a jid that is not a channel before any
/// stanza leaves. Measured live (2026-10-09): the server answered metadata with
/// 400 and never answered history, which then waited the library's 75 s IQ
/// timeout and came back as Unavailable.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn metadata_and_history_refuse_a_non_channel_jid_before_any_iq() {
    let mut f = fixture("metadata_and_history_refuse_a_non_channel_jid_before_any_iq").await;
    let (newsletter_iqs, mex_iqs) = (iqs_in(&f.mock, NEWSLETTER), iqs_in(&f.mock, MEX));
    let expected = format!("'{GROUP}' is not a channel: expected a jid ending in @newsletter");
    let metadata = f
        .channels
        .get_newsletter_metadata(f.jid_request(GROUP))
        .await
        .expect_err("not a channel");
    let history = f
        .channels
        .get_newsletter_messages(f.history(GROUP, 5, 0))
        .await
        .expect_err("not a channel");
    for status in [metadata, history] {
        assert_eq!(status.code(), Code::InvalidArgument, "{status:?}");
        assert_eq!(status.message(), expected);
    }
    assert_only_the_probe_was_sent(&mut f, newsletter_iqs, mex_iqs).await;
    f.cleanup().await;
}
