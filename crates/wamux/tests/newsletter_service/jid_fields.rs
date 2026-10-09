//! #121: every jid in `newsletters.proto` and the `JidRequest` two channel
//! RPCs share is a `pb::Jid`. Unset or empty answers "missing jid"; since #99
//! every RPC that names a channel refuses another server, with one message.

use tonic::Code;
use wamux::proto::v1 as pb;

use crate::captured::CHANNEL;
use crate::harness::fixture;

const GROUP: &str = "120363041234567890@g.us";

fn refused(case: &str, result: Result<(), tonic::Status>) -> String {
    let status = result.expect_err(case);
    assert_eq!(status.code(), Code::InvalidArgument, "{case}: {status:?}");
    status.message().to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_channel_rpc_answers_missing_jid_for_an_unset_or_empty_jid() {
    let mut f = fixture("every_channel_rpc_answers_missing_jid_for_an_unset_or_empty_jid").await;
    let unset = pb::JidRequest {
        account: Some(f.account.clone()),
        jid: None,
    };
    let metadata = f.channels.get_newsletter_metadata(unset.clone()).await;
    let live = f.channels.subscribe_newsletter_live_updates(unset).await;
    let history = f
        .channels
        .get_newsletter_messages(f.history("", 5, 0))
        .await;
    let add_ons = f.channels.get_my_newsletter_add_ons(f.add_ons("", 5)).await;
    let vote = f
        .channels
        .send_newsletter_poll_vote(f.vote("", 1, Vec::new()))
        .await;
    let results = [
        ("GetNewsletterMetadata: unset", metadata.map(drop)),
        ("SubscribeNewsletterLiveUpdates: unset", live.map(drop)),
        ("GetNewsletterMessages: empty", history.map(drop)),
        ("GetMyNewsletterAddOns: empty", add_ons.map(drop)),
        ("SendNewsletterPollVote: empty", vote.map(drop)),
    ];
    for (case, result) in results {
        assert_eq!(refused(case, result), "missing jid", "{case}");
    }
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_five_channel_rpcs_refuse_another_server() {
    let mut f = fixture("the_five_channel_rpcs_refuse_another_server").await;
    let expected = format!("'{GROUP}' is not a channel: expected a jid ending in @newsletter");
    let metadata = f
        .channels
        .get_newsletter_metadata(f.jid_request(GROUP))
        .await;
    let history = f
        .channels
        .get_newsletter_messages(f.history(GROUP, 5, 0))
        .await;
    let live = f
        .channels
        .subscribe_newsletter_live_updates(f.jid_request(GROUP))
        .await;
    let add_ons = f
        .channels
        .get_my_newsletter_add_ons(f.add_ons(GROUP, 5))
        .await;
    let vote = f
        .channels
        .send_newsletter_poll_vote(f.vote(GROUP, 1, vec![vec![0; 32]]))
        .await;
    for (case, result) in [
        ("GetNewsletterMetadata", metadata.map(drop)),
        ("GetNewsletterMessages", history.map(drop)),
        ("SubscribeNewsletterLiveUpdates", live.map(drop)),
        ("GetMyNewsletterAddOns", add_ons.map(drop)),
        ("SendNewsletterPollVote", vote.map(drop)),
    ] {
        assert_eq!(refused(case, result), expected, "{case}");
    }
    f.cleanup().await;
}

// The channel the server describes comes back as a `Jid`, verbatim.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_metadata_answers_the_channel_as_a_jid() {
    let mut f = fixture("get_metadata_answers_the_channel_as_a_jid").await;
    f.mock.answer_mex_with(crate::captured::METADATA_JSON);
    let channel = f
        .channels
        .get_newsletter_metadata(f.jid_request(CHANNEL))
        .await
        .expect("metadata")
        .into_inner();
    assert_eq!(
        channel.jid,
        Some(pb::Jid {
            value: CHANNEL.to_string()
        })
    );
    f.cleanup().await;
}
