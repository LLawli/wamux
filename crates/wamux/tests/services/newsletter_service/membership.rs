//! Following, muting and creating a channel (#199): each one is a MEX
//! mutation. Every test asserts the variables that reached the wire; the
//! answers wrap the channel captured on 2026-10-02 under the field each
//! mutation's parser reads (whatsapp-rust 6f07e3a).

use serde_json::{Value, json};
use tonic::Code;
use wacore_binary::NodeContent;
use wamux::proto::v1 as pb;

use crate::common::mock_wire::{iqs_in, operation, sent_iq};
use crate::newsletter_service::captured::{CHANNEL, METADATA_JSON};
use crate::newsletter_service::harness::{Fixture, MEX, NEWSLETTER, fixture};

const GROUP: &str = "120363041234567890@g.us";

/// The captured channel object, with this account's role set to `role`.
fn channel_with_role(role: &str) -> Value {
    let mut captured: Value = serde_json::from_str(METADATA_JSON).expect("captured JSON");
    let mut channel = captured["data"]["xwa2_newsletter"].take();
    channel["viewer_metadata"]["role"] = json!(role);
    channel
}

/// Answer every later MEX query with `value` under `field`, as the server does.
fn answer_mex(f: &Fixture, field: &str, value: Value) {
    f.mock
        .answer_mex_with(&json!({ "data": { field: value } }).to_string());
}

/// The `variables` of the MEX mutation the client sent last.
async fn sent_mex_variables(f: &Fixture) -> Value {
    let iq = sent_iq(&f.mock, MEX, "get", "query").await;
    let query = operation(&iq).expect("the iq carries a query");
    let bytes: Vec<u8> = match &query.content {
        Some(NodeContent::Bytes(bytes)) => bytes.clone(),
        Some(NodeContent::String(text)) => text.as_bytes().to_vec(),
        other => panic!("the MEX query carries no payload: {other:?}"),
    };
    let payload: Value = serde_json::from_slice(&bytes).expect("MEX payload is JSON");
    payload["variables"].clone()
}

fn mute(f: &Fixture, jid: &str, muted: bool) -> pb::NewsletterMuteRequest {
    pb::NewsletterMuteRequest {
        account: Some(f.account.clone()),
        jid: Some(pb::Jid {
            value: jid.to_string(),
        }),
        muted,
    }
}

fn create(f: &Fixture, name: &str, description: Option<&str>) -> pb::CreateNewsletterRequest {
    pb::CreateNewsletterRequest {
        account: Some(f.account.clone()),
        name: name.to_string(),
        description: description.map(str::to_string),
    }
}

/// Answers nothing (#199): the server's join answer lacks the channel's name,
/// badge and this account's role. A full channel object here proves the core
/// does not relay it even when the server sends one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn follow_newsletter_sends_the_join_and_answers_empty() {
    let mut f = fixture("follow_newsletter_sends_the_join_and_answers_empty").await;
    answer_mex(
        &f,
        "xwa2_newsletter_join_v2",
        channel_with_role("SUBSCRIBER"),
    );
    let answer: pb::Empty = f
        .channels
        .follow_newsletter(f.jid_request(CHANNEL))
        .await
        .expect("follow")
        .into_inner();
    assert_eq!(answer, pb::Empty {});
    assert_eq!(sent_mex_variables(&f).await["newsletter_id"], CHANNEL);
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unfollow_newsletter_sends_the_leave() {
    let mut f = fixture("unfollow_newsletter_sends_the_leave").await;
    answer_mex(&f, "xwa2_newsletter_leave_v2", json!({ "id": CHANNEL }));
    f.channels
        .unfollow_newsletter(f.jid_request(CHANNEL))
        .await
        .expect("unfollow");
    assert_eq!(sent_mex_variables(&f).await["newsletter_id"], CHANNEL);
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mute_newsletter_sends_admin_activity_on_and_off() {
    let mut f = fixture("mute_newsletter_sends_admin_activity_on_and_off").await;
    answer_mex(
        &f,
        "xwa2_newsletter_update_user_setting",
        json!({ "id": CHANNEL }),
    );
    for (muted, value) in [(true, "ON"), (false, "OFF")] {
        f.channels
            .mute_newsletter(mute(&f, CHANNEL, muted))
            .await
            .unwrap_or_else(|e| panic!("mute {muted}: {e}"));
        let input = sent_mex_variables(&f).await["input"].clone();
        assert_eq!(input["newsletter_id"], CHANNEL, "{muted}");
        assert_eq!(input["type"], "MUTE_ADMIN_ACTIVITY", "{muted}");
        assert_eq!(input["value"], value, "{muted}");
    }
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_newsletter_sends_the_name_and_description_and_relays_the_channel() {
    let mut f =
        fixture("create_newsletter_sends_the_name_and_description_and_relays_the_channel").await;
    answer_mex(&f, "xwa2_newsletter_create", channel_with_role("OWNER"));
    let channel = f
        .channels
        .create_newsletter(create(&f, "wamux 199", Some("a test channel")))
        .await
        .expect("create")
        .into_inner();
    let input = sent_mex_variables(&f).await["input"].clone();
    assert_eq!(input["name"], "wamux 199");
    assert_eq!(input["description"], "a test channel");
    assert_eq!(channel.jid.map(|jid| jid.value).as_deref(), Some(CHANNEL));
    assert_eq!(channel.role, pb::NewsletterRole::Owner as i32);
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_newsletter_without_a_description_sends_none() {
    let mut f = fixture("create_newsletter_without_a_description_sends_none").await;
    answer_mex(&f, "xwa2_newsletter_create", channel_with_role("OWNER"));
    f.channels
        .create_newsletter(create(&f, "wamux 199", None))
        .await
        .expect("create");
    let input = sent_mex_variables(&f).await["input"].clone();
    assert_eq!(input["name"], "wamux 199");
    assert!(input["description"].is_null(), "description: {input}");
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_newsletter_refuses_an_empty_name_before_the_wire() {
    let mut f = fixture("create_newsletter_refuses_an_empty_name_before_the_wire").await;
    let mex_before = iqs_in(&f.mock, MEX);
    let status = f
        .channels
        .create_newsletter(create(&f, "", Some("no name")))
        .await
        .expect_err("an empty name");
    assert_eq!(status.code(), Code::InvalidArgument, "{status:?}");
    assert_eq!(status.message(), "missing name");
    assert_eq!(iqs_in(&f.mock, MEX), mex_before, "no MEX query");
    f.cleanup().await;
}

/// #99: an RPC that names a channel refuses another server by name, before
/// anything is sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn follow_unfollow_and_mute_refuse_a_non_channel_jid_by_name() {
    let mut f = fixture("follow_unfollow_and_mute_refuse_a_non_channel_jid_by_name").await;
    let (mex_before, newsletter_before) = (iqs_in(&f.mock, MEX), iqs_in(&f.mock, NEWSLETTER));
    let expected = format!("'{GROUP}' is not a channel: expected a jid ending in @newsletter");
    let follow = f.channels.follow_newsletter(f.jid_request(GROUP)).await;
    let unfollow = f.channels.unfollow_newsletter(f.jid_request(GROUP)).await;
    let muted = f.channels.mute_newsletter(mute(&f, GROUP, true)).await;
    for (rpc, result) in [
        ("FollowNewsletter", follow.map(drop)),
        ("UnfollowNewsletter", unfollow.map(drop)),
        ("MuteNewsletter", muted.map(drop)),
    ] {
        let status = result.expect_err(rpc);
        assert_eq!(status.code(), Code::InvalidArgument, "{rpc}: {status:?}");
        assert_eq!(status.message(), expected, "{rpc}");
    }
    assert_eq!(iqs_in(&f.mock, MEX), mex_before, "no MEX query");
    assert_eq!(iqs_in(&f.mock, NEWSLETTER), newsletter_before, "no IQ");
    f.cleanup().await;
}

/// The server answered the mutation with a null field: nothing was applied.
/// The library reports that with no server code, so it reaches the edge as
/// Unavailable today. Pinned so a change to that mapping is a decision (#199
/// left it out of scope).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mutation_the_server_answered_with_no_data_answers_unavailable_today() {
    let mut f =
        fixture("a_mutation_the_server_answered_with_no_data_answers_unavailable_today").await;
    answer_mex(&f, "xwa2_newsletter_join_v2", Value::Null);
    let status = f
        .channels
        .follow_newsletter(f.jid_request(CHANNEL))
        .await
        .expect_err("not joined");
    assert_eq!(status.code(), Code::Unavailable, "{status:?}");
    // The mutation did go out: the refusal is the server's answer, not the core's.
    assert_eq!(sent_mex_variables(&f).await["newsletter_id"], CHANNEL);
    f.cleanup().await;
}
