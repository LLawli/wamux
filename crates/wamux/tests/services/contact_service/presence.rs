//! #271: UnsubscribePresence releases what SubscribePresence asked for. The
//! core relays both to the library; what reaches the wire is a `<presence>`
//! stanza each, with the type and the contact the edge named.

use tonic::Code;
use wacore_binary::Node;
use wamux::proto::v1 as pb;
use wamux::stress::MockWaServer;

use crate::common::mock_wire::attr;
use crate::common::{LOOPBACK_STEP_DEADLINE, poll_until};
use crate::contact_service::harness::{CONTACT_PN, Fixture, fixture, jid};

/// The `<presence>` stanzas of this `type` the client sent, oldest first. The
/// client also sends `type="available"` on its own after login, so a test
/// never counts presence stanzas without the type.
fn presences_of_type(mock: &MockWaServer, kind: &str) -> Vec<Node> {
    mock.client_stanzas("presence")
        .into_iter()
        .filter(|node| attr(node, "type").as_deref() == Some(kind))
        .collect()
}

/// Wait for the first `<presence type=kind>` to reach the mock.
async fn sent_presence(mock: &MockWaServer, kind: &str) -> Node {
    poll_until(
        &format!("a <presence type={kind}>"),
        LOOPBACK_STEP_DEADLINE,
        || async { presences_of_type(mock, kind).into_iter().next() },
    )
    .await
}

fn presence_request(f: &Fixture, value: Option<pb::Jid>) -> pb::SubscribePresenceRequest {
    pb::SubscribePresenceRequest {
        account: Some(f.account.clone()),
        jid: value,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsubscribe_presence_sends_an_unsubscribe_stanza() {
    let mut f = fixture("unsubscribe_presence_sends_an_unsubscribe_stanza").await;
    f.contacts
        .subscribe_presence(presence_request(&f, jid(CONTACT_PN)))
        .await
        .expect("subscribe");
    let subscribe = sent_presence(&f.mock, "subscribe").await;
    assert_eq!(attr(&subscribe, "to").as_deref(), Some(CONTACT_PN));

    f.contacts
        .unsubscribe_presence(presence_request(&f, jid(CONTACT_PN)))
        .await
        .expect("unsubscribe");
    let unsubscribe = sent_presence(&f.mock, "unsubscribe").await;
    assert_eq!(attr(&unsubscribe, "to").as_deref(), Some(CONTACT_PN));

    let order: Vec<Option<String>> = f
        .mock
        .client_stanzas("presence")
        .iter()
        .map(|node| attr(node, "type"))
        .filter(|kind| matches!(kind.as_deref(), Some("subscribe" | "unsubscribe")))
        .collect();
    assert_eq!(
        order,
        vec![Some("subscribe".into()), Some("unsubscribe".into())],
        "one subscribe, then one unsubscribe"
    );
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsubscribe_presence_without_a_jid_is_invalid_argument() {
    let mut f = fixture("unsubscribe_presence_without_a_jid_is_invalid_argument").await;
    let status = f
        .contacts
        .unsubscribe_presence(presence_request(&f, None))
        .await
        .expect_err("an unset jid never reaches the library");
    assert_eq!(status.code(), Code::InvalidArgument, "{status:?}");
    assert!(
        presences_of_type(&f.mock, "unsubscribe").is_empty(),
        "nothing reached the wire"
    );
    f.cleanup().await;
}
