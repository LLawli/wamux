//! Status codes every RPC shares (the account resolution), and the jid and
//! target checks most of them make. Each refusal test also proves nothing
//! reached the wire.

use tonic::Code;
use wamux::proto::v1 as pb;

use crate::common::{self, mock_wire::account_ref};
use crate::every_rpc::call_every_rpc;
use crate::harness::{Fixture, assert_only_the_probe_was_sent, fixture, jid, key, wire_count};

fn assert_every(results: &[(&str, Result<(), tonic::Status>)], code: Code) {
    assert_eq!(results.len(), 23, "all 23 RPCs");
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
        &call_every_rpc(&mut f.messages, &unknown).await,
        Code::NotFound,
    );
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_rpc_answers_failed_precondition_when_not_connected() {
    let mut f = fixture("every_rpc_answers_failed_precondition_when_not_connected").await;
    let prefix = common::test_prefix("messaging_service", "every_rpc_not_connected_idle");
    common::sweep_orphans(f.logged.registry.storage(), &prefix).await;
    let idle = f
        .logged
        .registry
        .create_account(Some(&format!("{prefix}idle")))
        .await
        .expect("idle account");
    let idle_ref = account_ref(&idle.uuid.to_string());
    assert_every(
        &call_every_rpc(&mut f.messages, &idle_ref).await,
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
        &call_every_rpc(&mut f.messages, &empty).await,
        Code::InvalidArgument,
    );
    f.cleanup().await;
}

/// The 15 RPCs that name a chat (`to` or `chat`), all with `chat` as it.
async fn chat_scoped(f: &mut Fixture, chat: Option<pb::Jid>) -> Vec<(&'static str, tonic::Status)> {
    let a = f.a();
    let m = &mut f.messages;
    let c = || chat.clone();
    let mark = || pb::MarkReadRequest {
        account: a.clone(),
        chat: c(),
        message_ids: vec!["3EB0A".into()],
        sender: None,
    };
    let media = vec![pb::SendMediaChunk {
        part: Some(pb::send_media_chunk::Part::Header(pb::SendMediaHeader {
            account: a.clone(),
            to: c(),
            media_type: pb::MediaType::Image as i32,
            ..Default::default()
        })),
    }];
    let reply = pb::SendInteractiveReplyRequest {
        account: a.clone(),
        to: c(),
        quote: Some(pb::QuoteContext {
            quoted: Some(key("x@s.whatsapp.net", "3EB0OFFER")),
            participant: None,
        }),
        quoted_message: Vec::new(),
        reply: Some(pb::send_interactive_reply_request::Reply::Button(
            pb::ButtonReply {
                selected_id: "opt-1".into(),
                display_text: "Sim".into(),
            },
        )),
    };
    let err = |r: Result<(), tonic::Status>| r.expect_err("a bad chat is refused");
    vec![
        (
            "SendText",
            err(m
                .send_text(pb::SendTextRequest {
                    account: a.clone(),
                    to: c(),
                    text: "oi".into(),
                    ..Default::default()
                })
                .await
                .map(drop)),
        ),
        (
            "SendMedia",
            err(m.send_media(tokio_stream::iter(media)).await.map(drop)),
        ),
        (
            "FetchMessageHistory",
            err(m
                .fetch_message_history(pb::FetchMessageHistoryRequest {
                    account: a.clone(),
                    chat: c(),
                    count: 5,
                    ..Default::default()
                })
                .await
                .map(drop)),
        ),
        (
            "SendPresence",
            err(m
                .send_presence(pb::SendPresenceRequest {
                    account: a.clone(),
                    chat: c(),
                    state: pb::PresenceState::Composing as i32,
                })
                .await
                .map(drop)),
        ),
        ("MarkRead", err(m.mark_read(mark()).await.map(drop))),
        (
            "MarkChatRead",
            err(m.mark_chat_read(mark()).await.map(drop)),
        ),
        ("MarkUnread", err(m.mark_unread(mark()).await.map(drop))),
        (
            "ArchiveChat",
            err(m
                .archive_chat(pb::ArchiveChatRequest {
                    account: a.clone(),
                    chat: c(),
                    archived: true,
                })
                .await
                .map(drop)),
        ),
        (
            "PinChat",
            err(m
                .pin_chat(pb::PinChatRequest {
                    account: a.clone(),
                    chat: c(),
                    pinned: true,
                })
                .await
                .map(drop)),
        ),
        (
            "MuteChat",
            err(m
                .mute_chat(pb::MuteChatRequest {
                    account: a.clone(),
                    chat: c(),
                    muted: false,
                    mute_until_ms: 0,
                })
                .await
                .map(drop)),
        ),
        (
            "DeleteChat",
            err(m
                .delete_chat(pb::DeleteChatRequest {
                    account: a.clone(),
                    chat: c(),
                    delete_media: false,
                })
                .await
                .map(drop)),
        ),
        (
            "SendContact",
            err(m
                .send_contact(pb::SendContactRequest {
                    account: a.clone(),
                    to: c(),
                    display_name: "Maria".into(),
                    vcard: String::new(),
                    quote: None,
                })
                .await
                .map(drop)),
        ),
        (
            "SendInteractiveReply",
            err(m.send_interactive_reply(reply).await.map(drop)),
        ),
        (
            "SendPoll",
            err(m
                .send_poll(pb::SendPollRequest {
                    account: a.clone(),
                    to: c(),
                    name: "?".into(),
                    options: vec!["a".into(), "b".into()],
                    selectable_count: 1,
                })
                .await
                .map(drop)),
        ),
        (
            "SendPollVote",
            err(m
                .send_poll_vote(pb::SendPollVoteRequest {
                    account: a.clone(),
                    chat: c(),
                    poll_id: "3EB0POLL".into(),
                    poll_creator: jid("5511900000002@s.whatsapp.net"),
                    message_secret: vec![7; 32],
                    options: Vec::new(),
                })
                .await
                .map(drop)),
        ),
    ]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_chat_rpc_rejects_a_missing_or_malformed_jid() {
    let mut f = fixture("every_chat_rpc_rejects_a_missing_or_malformed_jid").await;
    let before = wire_count(&f);
    for (case, chat) in [
        ("absent", None),
        ("empty", jid("")),
        ("malformed", jid("not a jid")),
    ] {
        let statuses = chat_scoped(&mut f, chat).await;
        assert_eq!(statuses.len(), 15, "every RPC that names a chat");
        for (rpc, status) in statuses {
            assert_eq!(
                status.code(),
                Code::InvalidArgument,
                "{rpc}, {case} jid: {status:?}"
            );
        }
    }
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}

/// The 5 calls that act on a message by its key, with `target` as the key.
async fn target_scoped(
    f: &mut Fixture,
    target: Option<pb::MessageKey>,
) -> Vec<(&'static str, tonic::Status)> {
    let a = f.a();
    let m = &mut f.messages;
    let t = || target.clone();
    let err = |r: Result<(), tonic::Status>| r.expect_err("a bad target is refused");
    vec![
        (
            "SendReaction",
            err(m
                .send_reaction(pb::SendReactionRequest {
                    account: a.clone(),
                    target: t(),
                    emoji: "x".into(),
                })
                .await
                .map(drop)),
        ),
        (
            "EditMessage",
            err(m
                .edit_message(pb::EditMessageRequest {
                    account: a.clone(),
                    target: t(),
                    new_text: "x".into(),
                })
                .await
                .map(drop)),
        ),
        (
            "DeleteMessage for everyone",
            err(m
                .delete_message(pb::DeleteMessageRequest {
                    account: a.clone(),
                    target: t(),
                    for_everyone: true,
                })
                .await
                .map(drop)),
        ),
        (
            "DeleteMessage for me",
            err(m
                .delete_message(pb::DeleteMessageRequest {
                    account: a.clone(),
                    target: t(),
                    for_everyone: false,
                })
                .await
                .map(drop)),
        ),
        (
            "StarMessage",
            err(m
                .star_message(pb::StarMessageRequest {
                    account: a.clone(),
                    target: t(),
                    starred: true,
                })
                .await
                .map(drop)),
        ),
    ]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_target_rpc_rejects_a_missing_or_malformed_target() {
    let mut f = fixture("every_target_rpc_rejects_a_missing_or_malformed_target").await;
    let before = wire_count(&f);
    for (case, target) in [
        ("absent", None),
        ("malformed chat", Some(key("not a jid", "3EB0X"))),
    ] {
        for (rpc, status) in target_scoped(&mut f, target).await {
            assert_eq!(
                status.code(),
                Code::InvalidArgument,
                "{rpc}, {case}: {status:?}"
            );
        }
    }
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}
