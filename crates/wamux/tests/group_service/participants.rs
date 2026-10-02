//! The six participant changes. The server answers per participant, so a
//! request can partially succeed; the core relays every verdict as it came
//! (one ok, one refused with the 403 `add_request` token), never one status.

use tonic::transport::Channel;
use wacore_binary::Node;
use wacore_binary::builder::NodeBuilder;
use wamux::proto::v1 as pb;
use wamux::proto::v1::group_service_client::GroupServiceClient;

use crate::captured::GROUP;
use crate::harness::{G2, attr, fixture, operation, participant_jids, sent_iq};

const ACCEPTED: &str = "5511900000003@s.whatsapp.net";
const REFUSED: &str = "5511900000004@s.whatsapp.net";

/// One verdict per participant: `ACCEPTED` went through (`type` = `ok_type`),
/// `REFUSED` got a 403 carrying an invite token.
fn verdicts(ok_type: &str) -> Vec<Node> {
    vec![
        NodeBuilder::new("participant")
            .attr("jid", ACCEPTED)
            .attr("type", ok_type)
            .build(),
        NodeBuilder::new("participant")
            .attr("jid", REFUSED)
            .attr("error", "403")
            .attr("phone_number", REFUSED)
            .children([NodeBuilder::new("add_request")
                .attr("code", "ABC123DEF")
                .attr("expiration", "1735689600")
                .build()])
            .build(),
    ]
}

fn expected(ok_type: &str) -> Vec<pb::ParticipantChange> {
    vec![
        pb::ParticipantChange {
            jid: ACCEPTED.into(),
            status: ok_type.into(),
            ..Default::default()
        },
        pb::ParticipantChange {
            jid: REFUSED.into(),
            error: "403".into(),
            phone_number: REFUSED.into(),
            add_request: Some(pb::AddRequestInfo {
                code: "ABC123DEF".into(),
                expiration: 1_735_689_600,
            }),
            ..Default::default()
        },
    ]
}

type Call = for<'a> fn(
    &'a mut GroupServiceClient<Channel>,
    pb::ParticipantsRequest,
) -> std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<tonic::Response<pb::ParticipantsResponse>, tonic::Status>,
            > + Send
            + 'a,
    >,
>;

/// Answer `action`, call the RPC for both participants, and check the relay
/// and the `<participant>` list the request put on the wire.
async fn relays_each_verdict(test: &str, call: Call, action: &'static str, ok_type: &str) {
    let mut f = fixture(test).await;
    f.mock.answer_iq(
        G2,
        "set",
        action,
        NodeBuilder::new(action).children(verdicts(ok_type)).build(),
    );
    let request = f.participants(&[ACCEPTED, REFUSED]);
    let answer = call(&mut f.groups, request)
        .await
        .expect(action)
        .into_inner();
    assert_eq!(answer.results, expected(ok_type));
    let iq = sent_iq(&f.mock, G2, "set", action).await;
    assert_eq!(attr(&iq, "to").as_deref(), Some(GROUP));
    assert_eq!(
        participant_jids(operation(&iq).unwrap()),
        [ACCEPTED, REFUSED]
    );
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn add_participants_relays_each_verdict() {
    relays_each_verdict(
        "add_participants_relays_each_verdict",
        |g, r| Box::pin(g.add_participants(r)),
        "add",
        "200",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_participants_relays_each_verdict() {
    relays_each_verdict(
        "remove_participants_relays_each_verdict",
        |g, r| Box::pin(g.remove_participants(r)),
        "remove",
        "200",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn promote_admins_relays_each_verdict() {
    relays_each_verdict(
        "promote_admins_relays_each_verdict",
        |g, r| Box::pin(g.promote_admins(r)),
        "promote",
        "admin",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn demote_admins_relays_each_verdict() {
    relays_each_verdict(
        "demote_admins_relays_each_verdict",
        |g, r| Box::pin(g.demote_admins(r)),
        "demote",
        "200",
    )
    .await;
}

/// Membership actions wrap the verdicts one level deeper:
/// `<membership_requests_action><approve|reject>`.
async fn membership_relays_each_verdict(test: &str, call: Call, action: &'static str) {
    let mut f = fixture(test).await;
    let inner = NodeBuilder::new(action).children(verdicts("200")).build();
    let answer_node = NodeBuilder::new("membership_requests_action")
        .children([inner])
        .build();
    f.mock
        .answer_iq(G2, "set", "membership_requests_action", answer_node);
    let request = f.participants(&[ACCEPTED, REFUSED]);
    let answer = call(&mut f.groups, request)
        .await
        .expect(action)
        .into_inner();
    assert_eq!(answer.results, expected("200"));
    let iq = sent_iq(&f.mock, G2, "set", "membership_requests_action").await;
    let verb = operation(&iq)
        .unwrap()
        .get_optional_child(action)
        .expect("the action under the wrapper");
    assert_eq!(participant_jids(verb), [ACCEPTED, REFUSED]);
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approve_membership_requests_relays_each_verdict() {
    membership_relays_each_verdict(
        "approve_membership_requests_relays_each_verdict",
        |g, r| Box::pin(g.approve_membership_requests(r)),
        "approve",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reject_membership_requests_relays_each_verdict() {
    membership_relays_each_verdict(
        "reject_membership_requests_relays_each_verdict",
        |g, r| Box::pin(g.reject_membership_requests(r)),
        "reject",
    )
    .await;
}
