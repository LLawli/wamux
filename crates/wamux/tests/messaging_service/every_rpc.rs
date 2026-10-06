//! Every one of the 23 RPCs, called for one account with arguments valid in
//! shape, for the tests that check a status all of them share. The name is
//! the proto RPC, so a failure says which one.

use tonic::transport::Channel;
use wamux::proto::v1 as pb;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;

use crate::harness::{PEER_PN_USER, jid, jids, key};

type Outcome = (&'static str, Result<(), tonic::Status>);

fn peer() -> String {
    format!("{PEER_PN_USER}@s.whatsapp.net")
}

pub async fn call_every_rpc(
    m: &mut MessagingServiceClient<Channel>,
    account: &pb::AccountRef,
) -> Vec<Outcome> {
    let mut out = sends(m, account).await;
    out.extend(reads_and_marks(m, account).await);
    out.extend(chat_actions(m, account).await);
    out.extend(rich(m, account).await);
    out.extend(statuses(m, account).await);
    out
}

async fn sends(m: &mut MessagingServiceClient<Channel>, account: &pb::AccountRef) -> Vec<Outcome> {
    let a = || Some(account.clone());
    let target = || Some(key(peer(), "3EB0TARGET"));
    let media = vec![
        pb::SendMediaChunk {
            part: Some(pb::send_media_chunk::Part::Header(pb::SendMediaHeader {
                account: a(),
                to: jid(peer()),
                media_type: pb::MediaType::Image as i32,
                ..Default::default()
            })),
        },
        pb::SendMediaChunk {
            part: Some(pb::send_media_chunk::Part::Chunk(b"jpeg".to_vec())),
        },
    ];
    vec![
        (
            "SendText",
            m.send_text(pb::SendTextRequest {
                account: a(),
                to: jid(peer()),
                text: "oi".into(),
                ..Default::default()
            })
            .await
            .map(drop),
        ),
        (
            "SendMedia",
            m.send_media(tokio_stream::iter(media)).await.map(drop),
        ),
        (
            "SendReaction",
            m.send_reaction(pb::SendReactionRequest {
                account: a(),
                target: target(),
                emoji: "\u{1F44D}".into(),
            })
            .await
            .map(drop),
        ),
        (
            "EditMessage",
            m.edit_message(pb::EditMessageRequest {
                account: a(),
                target: target(),
                new_text: "oi!".into(),
            })
            .await
            .map(drop),
        ),
        (
            "DeleteMessage",
            m.delete_message(pb::DeleteMessageRequest {
                account: a(),
                target: target(),
                for_everyone: true,
            })
            .await
            .map(drop),
        ),
    ]
}

async fn reads_and_marks(
    m: &mut MessagingServiceClient<Channel>,
    account: &pb::AccountRef,
) -> Vec<Outcome> {
    let a = || Some(account.clone());
    let mark = || pb::MarkReadRequest {
        account: a(),
        chat: jid(peer()),
        message_ids: vec!["3EB0TARGET".into()],
        sender: None,
    };
    vec![
        (
            "FetchMessageHistory",
            m.fetch_message_history(pb::FetchMessageHistoryRequest {
                account: a(),
                chat: jid(peer()),
                oldest_msg_id: "3EB0OLDEST".into(),
                count: 50,
                ..Default::default()
            })
            .await
            .map(drop),
        ),
        (
            "SendPresence",
            m.send_presence(pb::SendPresenceRequest {
                account: a(),
                chat: jid(peer()),
                state: pb::PresenceState::Composing as i32,
            })
            .await
            .map(drop),
        ),
        ("MarkRead", m.mark_read(mark()).await.map(drop)),
        ("MarkChatRead", m.mark_chat_read(mark()).await.map(drop)),
        ("MarkUnread", m.mark_unread(mark()).await.map(drop)),
    ]
}

async fn chat_actions(
    m: &mut MessagingServiceClient<Channel>,
    account: &pb::AccountRef,
) -> Vec<Outcome> {
    let a = || Some(account.clone());
    let chat = || jid(peer());
    vec![
        (
            "StarMessage",
            m.star_message(pb::StarMessageRequest {
                account: a(),
                target: Some(key(peer(), "3EB0TARGET")),
                starred: true,
            })
            .await
            .map(drop),
        ),
        (
            "ArchiveChat",
            m.archive_chat(pb::ArchiveChatRequest {
                account: a(),
                chat: chat(),
                archived: true,
            })
            .await
            .map(drop),
        ),
        (
            "PinChat",
            m.pin_chat(pb::PinChatRequest {
                account: a(),
                chat: chat(),
                pinned: true,
            })
            .await
            .map(drop),
        ),
        (
            "MuteChat",
            m.mute_chat(pb::MuteChatRequest {
                account: a(),
                chat: chat(),
                muted: false,
                mute_until_ms: 0,
            })
            .await
            .map(drop),
        ),
        (
            "DeleteChat",
            m.delete_chat(pb::DeleteChatRequest {
                account: a(),
                chat: chat(),
                delete_media: false,
            })
            .await
            .map(drop),
        ),
    ]
}

async fn rich(m: &mut MessagingServiceClient<Channel>, account: &pb::AccountRef) -> Vec<Outcome> {
    let a = || Some(account.clone());
    let reply = pb::SendInteractiveReplyRequest {
        account: a(),
        to: jid(peer()),
        quote: Some(pb::QuoteContext {
            quoted: Some(key(peer(), "3EB0OFFER")),
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
    let secret = || vec![7u8; 32];
    let options = || vec!["azul".to_string(), "verde".to_string()];
    vec![
        (
            "SendContact",
            m.send_contact(pb::SendContactRequest {
                account: a(),
                to: jid(peer()),
                display_name: "Maria".into(),
                vcard: "BEGIN:VCARD\nEND:VCARD".into(),
                quote: None,
            })
            .await
            .map(drop),
        ),
        (
            "SendInteractiveReply",
            m.send_interactive_reply(reply).await.map(drop),
        ),
        (
            "SendPoll",
            m.send_poll(pb::SendPollRequest {
                account: a(),
                to: jid(peer()),
                name: "Qual cor?".into(),
                options: options(),
                selectable_count: 1,
            })
            .await
            .map(drop),
        ),
        (
            "SendPollVote",
            m.send_poll_vote(pb::SendPollVoteRequest {
                account: a(),
                chat: jid(peer()),
                poll_id: "3EB0POLL".into(),
                poll_creator: jid(peer()),
                message_secret: secret(),
                options: vec!["azul".into()],
            })
            .await
            .map(drop),
        ),
        (
            "AggregatePollVotes",
            m.aggregate_poll_votes(pb::AggregatePollVotesRequest {
                account: a(),
                poll_id: "3EB0POLL".into(),
                poll_creator: jid(peer()),
                message_secret: secret(),
                options: options(),
                votes: Vec::new(),
            })
            .await
            .map(drop),
        ),
    ]
}

async fn statuses(
    m: &mut MessagingServiceClient<Channel>,
    account: &pb::AccountRef,
) -> Vec<Outcome> {
    let a = || Some(account.clone());
    let recipients = || jids(&[peer()]);
    let media = vec![
        pb::PostStatusMediaChunk {
            part: Some(pb::post_status_media_chunk::Part::Header(
                pb::PostStatusMediaHeader {
                    account: a(),
                    media_type: pb::MediaType::Image as i32,
                    recipients: recipients(),
                    ..Default::default()
                },
            )),
        },
        pb::PostStatusMediaChunk {
            part: Some(pb::post_status_media_chunk::Part::Chunk(b"jpeg".to_vec())),
        },
    ];
    vec![
        (
            "PostStatusText",
            m.post_status_text(pb::PostStatusTextRequest {
                account: a(),
                text: "oi".into(),
                background_argb: 0,
                font: 1,
                recipients: recipients(),
            })
            .await
            .map(drop),
        ),
        (
            "PostStatusMedia",
            m.post_status_media(tokio_stream::iter(media))
                .await
                .map(drop),
        ),
        (
            "RevokeStatus",
            m.revoke_status(pb::RevokeStatusRequest {
                account: a(),
                message_id: "3EB0STATUS".into(),
                recipients: recipients(),
            })
            .await
            .map(drop),
        ),
    ]
}
