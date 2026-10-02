//! MarkRead puts a read receipt on the wire (the stanza that turns the
//! sender's ticks blue, #20); SendPresence a `<presence>` or a `<chatstate>`.

use tonic::Code;
use wacore_binary::Node;
use wamux::proto::v1 as pb;

use crate::common::mock_wire::{attr, operation};
use crate::harness::{
    Fixture, GROUP, assert_only_the_probe_was_sent, fixture, jid, newest_stanza, wire_count,
};

fn mark_read(f: &Fixture, chat: &str, sender: &str, ids: &[&str]) -> pb::MarkReadRequest {
    pb::MarkReadRequest {
        account: f.a(),
        chat: jid(chat),
        message_ids: ids.iter().map(|id| id.to_string()).collect(),
        sender: jid(sender),
    }
}

/// The ids under `<list><item id/>`, in order.
fn listed_ids(receipt: &Node) -> Vec<String> {
    receipt
        .get_optional_child_by_tag(&["list"])
        .map(|list| {
            list.get_children_by_tag("item")
                .filter_map(|i| attr(i, "id"))
                .collect()
        })
        .unwrap_or_default()
}

// One receipt acknowledges every id: the first on the stanza, the rest
// listed. In a group the author rides as `participant`; in a DM the author is
// the chat, so an empty `sender` is absence, not an error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mark_read_sends_one_read_receipt_for_the_ids() {
    let mut f = fixture("mark_read_sends_one_read_receipt_for_the_ids").await;
    let peer = f.peer.pn().to_string();
    let author = f.peer.lid().to_string();
    let cases = [
        (peer.as_str(), "", None),
        (GROUP, author.as_str(), Some(author.as_str())),
    ];
    for (chat, sender, participant) in cases {
        let before = f.mock.client_stanzas("receipt").len();
        let request = mark_read(&f, chat, sender, &["3EB0A", "3EB0B", "3EB0C"]);
        f.messages.mark_read(request).await.expect("mark read");
        let receipt = newest_stanza(&f.mock, "receipt", before).await;
        assert_eq!(attr(&receipt, "to").as_deref(), Some(chat));
        assert_eq!(attr(&receipt, "type").as_deref(), Some("read"));
        assert_eq!(attr(&receipt, "id").as_deref(), Some("3EB0A"));
        assert_eq!(
            attr(&receipt, "participant").as_deref(),
            participant,
            "{chat}"
        );
        assert_eq!(listed_ids(&receipt), vec!["3EB0B", "3EB0C"]);
    }
    f.cleanup().await;
}

// No ids, nothing to acknowledge: Ok, and no stanza.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mark_read_with_no_ids_sends_nothing() {
    let mut f = fixture("mark_read_with_no_ids_sends_nothing").await;
    let before = wire_count(&f);
    let request = mark_read(&f, &f.peer.pn().to_string(), "", &[]);
    f.messages
        .mark_read(request)
        .await
        .expect("an empty mark read");
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}

fn presence(f: &Fixture, state: &str) -> pb::SendPresenceRequest {
    pb::SendPresenceRequest {
        account: f.a(),
        chat: jid(f.peer.pn()),
        state: state.into(),
    }
}

// available/unavailable are the account's own `<presence>`, named; the
// typing states are a `<chatstate>` to the chat: composing, recording (an
// audio composing) and paused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_presence_sends_each_state() {
    let mut f = fixture("send_presence_sends_each_state").await;
    for state in ["available", "unavailable"] {
        let before = f.mock.client_stanzas("presence").len();
        f.messages
            .send_presence(presence(&f, state))
            .await
            .expect(state);
        let sent = newest_stanza(&f.mock, "presence", before).await;
        assert_eq!(attr(&sent, "type").as_deref(), Some(state));
        assert_eq!(attr(&sent, "name").as_deref(), Some("Stress"), "{state}");
    }
    let peer = f.peer.pn().to_string();
    let typing = [
        ("composing", "composing", None),
        ("recording", "composing", Some("audio")),
        ("paused", "paused", None),
    ];
    for (state, tag, media) in typing {
        let before = f.mock.client_stanzas("chatstate").len();
        f.messages
            .send_presence(presence(&f, state))
            .await
            .expect(state);
        let sent = newest_stanza(&f.mock, "chatstate", before).await;
        assert_eq!(attr(&sent, "to").as_deref(), Some(peer.as_str()), "{state}");
        let child = operation(&sent).expect("a chatstate child");
        assert_eq!(child.tag.as_ref(), tag, "{state}");
        assert_eq!(attr(child, "media").as_deref(), media, "{state}");
    }
    f.cleanup().await;
}

// The state is one of five exact tokens; nothing else reaches the wire.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_presence_state_is_invalid_argument() {
    let mut f = fixture("an_unknown_presence_state_is_invalid_argument").await;
    let before = wire_count(&f);
    for state in ["typing", "", "Available"] {
        let status = f
            .messages
            .send_presence(presence(&f, state))
            .await
            .expect_err(state);
        assert_eq!(
            status.code(),
            Code::InvalidArgument,
            "{state:?}: {status:?}"
        );
    }
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}
