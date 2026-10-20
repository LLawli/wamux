//! #197: SetAbout sets the account's own about text with one
//! `<iq xmlns="status" type="set"><status>` and relays the text as given.

use wacore_binary::{Node, NodeContent};
use wamux::proto::v1 as pb;

use crate::common::mock_wire::sent_iq;
use crate::harness::{Fixture, fixture};

fn set_about(f: &Fixture, about: &str) -> pb::SetAboutRequest {
    pb::SetAboutRequest {
        account: Some(f.account.clone()),
        about: about.to_string(),
    }
}

/// The text inside `<status>`, "" when the node carries none.
fn status_text(iq: &Node) -> String {
    let status = iq
        .get_optional_child("status")
        .expect("<status> under the iq");
    match &status.content {
        Some(NodeContent::String(text)) => text.to_string(),
        Some(NodeContent::Bytes(bytes)) => String::from_utf8_lossy(bytes).into_owned(),
        _ => String::new(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_about_sends_the_status_iq() {
    let mut f = fixture("set_about_sends_the_status_iq").await;
    f.contacts
        .set_about(set_about(&f, "Disponível das 8h às 18h"))
        .await
        .expect("set about");
    let iq = sent_iq(&f.mock, "status", "set", "status").await;
    assert_eq!(status_text(&iq), "Disponível das 8h às 18h");
    f.cleanup().await;
}

/// Whether "" clears the about is the server's call; the core does not add a
/// rule the library does not have.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_about_relays_empty_text() {
    let mut f = fixture("set_about_relays_empty_text").await;
    f.contacts
        .set_about(set_about(&f, ""))
        .await
        .expect("set an empty about");
    let iq = sent_iq(&f.mock, "status", "set", "status").await;
    assert_eq!(status_text(&iq), "");
    f.cleanup().await;
}
