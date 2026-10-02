//! The chat actions are app-state mutations: nothing goes to the chat, a patch
//! goes to the account's own devices. Each test opens that patch with the key
//! the store was seeded with and asserts the mutation, not a receipt.

use tonic::Code;
use wacore::appstate::Mutation;
use wamux::proto::v1 as pb;
use whatsapp_rust::waproto::whatsapp as wa;

use crate::harness::{
    Fixture, assert_only_the_probe_was_sent, fixture, jid, key, patches_sent, sent_patch,
    wire_count,
};

/// The one mutation of the patch sent after the first `after`, in `collection`.
/// `after` is counted before the RPC, so the patch it waits for is the RPC's.
async fn mutation_after(f: &Fixture, collection: &str, after: usize) -> Mutation {
    let patch = sent_patch(&f.mock, after).await;
    assert_eq!(patch.collection, collection);
    assert_eq!(patch.mutations.len(), 1, "{:?}", patch.mutations);
    let mutation = patch.mutations.into_iter().next().expect("one mutation");
    assert_eq!(mutation.operation, wa::syncd_mutation::SyncdOperation::SET);
    mutation
}

fn value(mutation: &Mutation) -> &wa::SyncActionValue {
    mutation
        .action_value
        .as_ref()
        .expect("a SET carries a value")
}

fn index(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

fn mark(f: &Fixture) -> pb::MarkReadRequest {
    pb::MarkReadRequest {
        account: f.a(),
        chat: jid(f.peer.pn()),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mark_chat_read_and_mark_unread_send_their_mutations() {
    let mut f = fixture("mark_chat_read_and_mark_unread_send_their_mutations").await;
    let chat = f.peer.pn().to_string();
    let sent = patches_sent(&f.mock);
    f.messages.mark_chat_read(mark(&f)).await.expect("read");
    let read = mutation_after(&f, "regular_low", sent).await;
    assert_eq!(read.index, index(&["markChatAsRead", &chat]));
    let action = value(&read)
        .mark_chat_as_read_action
        .as_option()
        .expect("action");
    assert_eq!(action.read, Some(true));

    let sent = patches_sent(&f.mock);
    f.messages.mark_unread(mark(&f)).await.expect("unread");
    let unread = mutation_after(&f, "regular_low", sent).await;
    let action = value(&unread)
        .mark_chat_as_read_action
        .as_option()
        .expect("action");
    assert_eq!(action.read, Some(false));
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn star_message_sends_star_and_unstar() {
    let mut f = fixture("star_message_sends_star_and_unstar").await;
    let chat = f.peer.pn().to_string();
    for starred in [true, false] {
        let request = pb::StarMessageRequest {
            account: f.a(),
            target: Some(key(&chat, "3EB0STAR")),
            starred,
        };
        let sent = patches_sent(&f.mock);
        f.messages.star_message(request).await.expect("star");
        let star = mutation_after(&f, "regular_high", sent).await;
        assert_eq!(star.index, index(&["star", &chat, "3EB0STAR", "1", "0"]));
        let action = value(&star).star_action.as_option().expect("action");
        assert_eq!(action.starred, Some(starred));
    }
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn archive_chat_sends_archive_and_unarchive() {
    let mut f = fixture("archive_chat_sends_archive_and_unarchive").await;
    let chat = f.peer.pn().to_string();
    for archived in [true, false] {
        let request = pb::ArchiveChatRequest {
            account: f.a(),
            chat: jid(&chat),
            archived,
        };
        let sent = patches_sent(&f.mock);
        f.messages.archive_chat(request).await.expect("archive");
        let archive = mutation_after(&f, "regular_low", sent).await;
        assert_eq!(archive.index, index(&["archive", &chat]));
        let action = value(&archive)
            .archive_chat_action
            .as_option()
            .expect("action");
        assert_eq!(action.archived, Some(archived));
    }
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pin_chat_sends_pin_and_unpin() {
    let mut f = fixture("pin_chat_sends_pin_and_unpin").await;
    let chat = f.peer.pn().to_string();
    for pinned in [true, false] {
        let request = pb::PinChatRequest {
            account: f.a(),
            chat: jid(&chat),
            pinned,
        };
        let sent = patches_sent(&f.mock);
        f.messages.pin_chat(request).await.expect("pin");
        let pin = mutation_after(&f, "regular_low", sent).await;
        assert_eq!(pin.index, index(&["pin_v1", &chat]));
        let action = value(&pin).pin_action.as_option().expect("action");
        assert_eq!(action.pinned, Some(pinned));
    }
    f.cleanup().await;
}

fn mute(f: &Fixture, muted: bool, mute_until_ms: i64) -> pb::MuteChatRequest {
    pb::MuteChatRequest {
        account: f.a(),
        chat: jid(f.peer.pn()),
        muted,
        mute_until_ms,
    }
}

/// Now plus a day, in ms: a mute that has not run out.
fn a_day_from_now_ms() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after the epoch");
    now.as_millis() as i64 + 86_400_000
}

// Muted with a deadline carries it; muted with 0 is forever; unmuted is
// muted=false whatever the deadline says.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mute_chat_sends_until_forever_and_unmute() {
    let mut f = fixture("mute_chat_sends_until_forever_and_unmute").await;
    let chat = f.peer.pn().to_string();
    let until = a_day_from_now_ms();
    let cases = [
        (true, until, Some(until)),
        (true, 0, None),
        (false, until, None),
    ];
    for (muted, mute_until_ms, end) in cases {
        let sent = patches_sent(&f.mock);
        f.messages
            .mute_chat(mute(&f, muted, mute_until_ms))
            .await
            .expect("mute");
        let mutation = mutation_after(&f, "regular_high", sent).await;
        assert_eq!(mutation.index, index(&["mute", &chat]));
        let action = value(&mutation).mute_action.as_option().expect("action");
        assert_eq!(
            action.muted,
            Some(muted),
            "muted {muted} until {mute_until_ms}"
        );
        if let Some(end) = end {
            assert_eq!(action.mute_end_timestamp, Some(end));
        }
    }
    f.cleanup().await;
}

/// #101: a deadline already past is refused by the library, which the core
/// relays as Unavailable. Nothing is sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mute_chat_in_the_past_is_unavailable_today() {
    let mut f = fixture("mute_chat_in_the_past_is_unavailable_today").await;
    let before = wire_count(&f);
    let status = f
        .messages
        .mute_chat(mute(&f, true, 1_000))
        .await
        .expect_err("a deadline in 1970");
    assert_eq!(status.code(), Code::Unavailable, "{status:?}");
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_chat_sends_its_mutation() {
    let mut f = fixture("delete_chat_sends_its_mutation").await;
    let chat = f.peer.pn().to_string();
    let request = pb::DeleteChatRequest {
        account: f.a(),
        chat: jid(&chat),
        delete_media: true,
    };
    let sent = patches_sent(&f.mock);
    f.messages.delete_chat(request).await.expect("delete chat");
    let delete = mutation_after(&f, "regular_high", sent).await;
    assert_eq!(delete.index, index(&["deleteChat", &chat, "1"]));
    assert!(value(&delete).delete_chat_action.as_option().is_some());
    f.cleanup().await;
}

// Delete-for-me is a mutation, not a message: the answer is the target's key
// and carries no fan-out, because nothing was sent to the chat.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_message_for_me_sends_its_mutation() {
    let mut f = fixture("delete_message_for_me_sends_its_mutation").await;
    let chat = f.peer.pn().to_string();
    let messages_before = f.mock.client_messages().len();
    let target = key(&chat, "3EB0MINE");
    let request = pb::DeleteMessageRequest {
        account: f.a(),
        target: Some(target.clone()),
        for_everyone: false,
    };
    let sent = patches_sent(&f.mock);
    let answer = f.messages.delete_message(request).await.expect("delete");
    let answer = answer.into_inner();
    assert_eq!(answer.key, Some(target));
    assert_eq!(answer.recipient_fanout, None);
    let delete = mutation_after(&f, "regular_high", sent).await;
    assert_eq!(
        delete.index,
        index(&["deleteMessageForMe", &chat, "3EB0MINE", "1", "0"])
    );
    let action = value(&delete).delete_message_for_me_action.as_option();
    assert!(action.is_some(), "{delete:?}");
    assert_eq!(f.mock.client_messages().len(), messages_before);
    f.cleanup().await;
}
