//! The five reads, answered with what WhatsApp's server sent on 2026-10-02
//! (`captured`), relayed through the socket by value.

use serde_json::{Value, json};
use wamux::proto::v1 as pb;

use crate::captured::{
    self, GROUP, GROUP_ID, INVITE_CODE, OWNER_LID, OWNER_PN, REQUESTER_LID, SUBJECT,
};
use crate::harness::{G2, attr, fixture, operation, sent_iq};

/// What GetGroupMetadata, PreviewInvite and ListGroups relay for the captured
/// group: the roster keeps the `@lid` jid AND the phone number beside it, and
/// absent fields relay as `null`, never guessed (issue #1).
fn captured_group_json() -> Value {
    json!({
        "id": GROUP,
        "subject": SUBJECT,
        "description": null,
        "addressing_mode": "lid",
        "participants": [{
            "jid": OWNER_LID,
            "phone_number": OWNER_PN,
            "lid": null,
            "username": null,
            "type": "superadmin",
        }],
    })
}

fn json_of(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("relayed metadata is JSON")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_group_metadata_relays_the_captured_roster() {
    let mut f = fixture("get_group_metadata_relays_the_captured_roster").await;
    f.mock.answer_iq(G2, "get", "query", captured::metadata());
    let answer = f
        .groups
        .get_group_metadata(f.group_ref())
        .await
        .expect("metadata")
        .into_inner();
    assert_eq!(json_of(&answer.metadata), captured_group_json());
    let iq = sent_iq(&f.mock, G2, "get", "query").await;
    assert_eq!(attr(&iq, "to").as_deref(), Some(GROUP));
    assert_eq!(
        attr(operation(&iq).unwrap(), "request").as_deref(),
        Some("interactive")
    );
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_invite_link_relays_the_link() {
    let mut f = fixture("get_invite_link_relays_the_link").await;
    f.mock.answer_iq(G2, "get", "invite", captured::invite());
    let answer = f
        .groups
        .get_invite_link(f.group_ref())
        .await
        .expect("link")
        .into_inner();
    assert_eq!(
        answer.link,
        format!("https://chat.whatsapp.com/{INVITE_CODE}")
    );
    let iq = sent_iq(&f.mock, G2, "get", "invite").await;
    assert_eq!(
        attr(&iq, "to").as_deref(),
        Some(GROUP),
        "get, not set: no reset"
    );
    f.cleanup().await;
}

/// Pins the shape #96 questions: each `jid` relays as the library's `Jid`
/// struct, not a string, and the server's `phone_number` and `request_method`
/// are dropped.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_membership_requests_relays_the_captured_list() {
    let mut f = fixture("get_membership_requests_relays_the_captured_list").await;
    f.mock.answer_iq(
        G2,
        "get",
        "membership_approval_requests",
        captured::membership_requests(),
    );
    let answer = f
        .groups
        .get_membership_requests(f.group_ref())
        .await
        .expect("requests")
        .into_inner();
    let user = REQUESTER_LID.trim_end_matches("@lid");
    let expected = json!([{
        "jid": {"user": user, "server": "lid", "agent": 0, "device": 0, "integrator": 0},
        "request_time": 1_790_947_901u64,
    }]);
    assert_eq!(json_of(&answer.requests), expected);
    let iq = sent_iq(&f.mock, G2, "get", "membership_approval_requests").await;
    assert_eq!(attr(&iq, "to").as_deref(), Some(GROUP));
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_invite_relays_the_captured_metadata() {
    let mut f = fixture("preview_invite_relays_the_captured_metadata").await;
    f.mock
        .answer_iq(G2, "get", "invite", captured::invite_preview());
    let request = pb::PreviewInviteRequest {
        account: Some(f.account.clone()),
        code: INVITE_CODE.into(),
    };
    let answer = f
        .groups
        .preview_invite(request)
        .await
        .expect("preview")
        .into_inner();
    assert_eq!(json_of(&answer.metadata), captured_group_json());
    let iq = sent_iq(&f.mock, G2, "get", "invite").await;
    assert_eq!(
        attr(operation(&iq).unwrap(), "code").as_deref(),
        Some(INVITE_CODE)
    );
    assert!(
        attr(&iq, "to").is_some_and(|to| to.ends_with("g.us") && !to.contains(GROUP_ID)),
        "a preview asks g.us, not the group it does not belong to yet"
    );
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_participating_relays_the_captured_group() {
    let mut f = fixture("list_participating_relays_the_captured_group").await;
    f.mock
        .answer_iq(G2, "get", "participating", captured::participating());
    let answer = f
        .groups
        .list_participating(f.account.clone())
        .await
        .expect("list")
        .into_inner();
    assert_eq!(answer.groups.len(), 1);
    let summary = &answer.groups[0];
    assert_eq!(summary.jid, GROUP);
    assert_eq!(summary.subject, SUBJECT);
    assert_eq!(summary.participants, 1);
    assert_eq!(json_of(&summary.metadata), captured_group_json());
    let iq = sent_iq(&f.mock, G2, "get", "participating").await;
    let asked: Vec<&str> = operation(&iq)
        .unwrap()
        .children()
        .unwrap_or(&[])
        .iter()
        .map(|c| c.tag.as_ref())
        .collect();
    assert_eq!(
        asked,
        ["participants", "description"],
        "the full roster, not the slim overview"
    );
    f.cleanup().await;
}
