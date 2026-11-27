//! DeleteMessage for everyone revokes as a group admin when the target key is
//! someone else's (#276): the protocol key the members open is the key the
//! edge sent, `fromMe=false` and the author, never rewritten into a sender
//! revoke of the account's own message.

use tonic::Code;
use wamux::proto::v1 as pb;
use whatsapp_rust::waproto::whatsapp as wa;

use crate::common::mock_wire::{account_ref, attr};
use crate::messaging_service::harness::{
    Fixture, GROUP, assert_only_the_probe_was_sent, fixture, jid, key, newest_stanza, sender,
    wire_count,
};

const THEIRS: &str = "3EB0THEIRS276";

/// A message in `chat` that `author` wrote, not the account.
fn written_by(chat: &str, author: Option<String>) -> pb::MessageKey {
    pb::MessageKey {
        from_me: false,
        participant: author.and_then(jid),
        ..key(chat, THEIRS)
    }
}

fn revoke(account: Option<pb::AccountRef>, target: pb::MessageKey) -> pb::DeleteMessageRequest {
    pb::DeleteMessageRequest {
        account,
        target: Some(target),
        for_everyone: true,
    }
}

/// Revoke `target` for everyone and return the revoke stanza and the answer.
async fn revoke_on_the_wire(
    f: &mut Fixture,
    target: &pb::MessageKey,
) -> (wacore_binary::Node, pb::SendResult) {
    let before = f.mock.client_messages().len();
    let answer = f
        .messages
        .delete_message(revoke(f.a(), target.clone()))
        .await
        .expect("revoke")
        .into_inner();
    (newest_stanza(&f.mock, "message", before).await, answer)
}

/// The protocol message every group member opens from the revoke stanza.
async fn opened_revokes(f: &Fixture, stanza: &wacore_binary::Node) -> Vec<wa::MessageKey> {
    let mut keys = Vec::new();
    for member in [&f.peer, &f.other] {
        let opened = member.open_group(stanza, &sender()).await.expect("open");
        let protocol = opened
            .protocol_message
            .as_option()
            .expect("protocolMessage")
            .clone();
        assert_eq!(
            protocol.r#type,
            Some(wa::message::protocol_message::Type::Revoke)
        );
        keys.push(protocol.key.as_option().expect("the target key").clone());
    }
    keys
}

// The admin form is `edit="8"`, and its key names the author with
// `fromMe=false`. The answer is the target's own key, as for a sender revoke.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_for_everyone_of_anothers_group_message_goes_as_admin() {
    let mut f = fixture("delete_for_everyone_of_anothers_group_message_goes_as_admin").await;
    let author = f.peer.pn().to_string();
    let target = written_by(GROUP, Some(author.clone()));
    let (stanza, answer) = revoke_on_the_wire(&mut f, &target).await;
    assert_eq!(answer.key, Some(target.clone()));
    assert_eq!(attr(&stanza, "to").as_deref(), Some(GROUP));
    assert_eq!(attr(&stanza, "edit").as_deref(), Some("8"));
    for opened in opened_revokes(&f, &stanza).await {
        assert_eq!(opened.remote_jid.as_deref(), Some(GROUP));
        assert_eq!(opened.id.as_deref(), Some(THEIRS));
        assert_eq!(opened.from_me, Some(false));
        assert_eq!(opened.participant.as_deref(), Some(author.as_str()));
    }
    f.cleanup().await;
}

// The account's own group message is still a sender revoke: `edit="7"`,
// `fromMe=true` and no participant, exactly as before #276.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_for_everyone_of_own_group_message_stays_a_sender_revoke() {
    let mut f = fixture("delete_for_everyone_of_own_group_message_stays_a_sender_revoke").await;
    let target = key(GROUP, "3EB0MINE276");
    let (stanza, answer) = revoke_on_the_wire(&mut f, &target).await;
    assert_eq!(answer.key, Some(target.clone()));
    assert_eq!(attr(&stanza, "edit").as_deref(), Some("7"));
    for opened in opened_revokes(&f, &stanza).await {
        assert_eq!(opened.id.as_deref(), Some("3EB0MINE276"));
        assert_eq!(opened.from_me, Some(true));
        assert_eq!(opened.participant, None);
    }
    f.cleanup().await;
}

/// The two keys no revoke can be built from: another's message outside a
/// group, and another's group message with no author.
fn unrevokable(f: &Fixture) -> Vec<(&'static str, pb::MessageKey)> {
    vec![
        (
            "not mine, in a DM",
            written_by(f.peer.pn().to_string().as_str(), None),
        ),
        ("not mine, no participant", written_by(GROUP, None)),
    ]
}

// The shape of the key is about the request, so it is refused before the
// account is looked up (#41, #101): InvalidArgument, never NotFound.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admin_revoke_shapes_are_refused_before_the_account() {
    let mut f = fixture("admin_revoke_shapes_are_refused_before_the_account").await;
    let unknown = account_ref(&uuid::Uuid::new_v4().to_string());
    for (case, target) in unrevokable(&f) {
        let result = f
            .messages
            .delete_message(revoke(Some(unknown.clone()), target))
            .await;
        let status = result.expect_err(case);
        assert_eq!(status.code(), Code::InvalidArgument, "{case}: {status:?}");
        assert!(status.message().contains(THEIRS), "{case}: {status:?}");
    }
    f.cleanup().await;
}

// On a real account the same keys are refused and nothing reaches the wire:
// before #276 each one went out as a sender revoke of a different message.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_admin_revoke_sends_nothing() {
    let mut f = fixture("refused_admin_revoke_sends_nothing").await;
    let before = wire_count(&f);
    for (case, target) in unrevokable(&f) {
        let result = f.messages.delete_message(revoke(f.a(), target)).await;
        let status = result.expect_err(case);
        assert_eq!(status.code(), Code::InvalidArgument, "{case}: {status:?}");
    }
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}
