//! The sends to WAMUX_LIVE_DEST. A text and an image pass only on `delivered`;
//! a reaction, an edit and a revoke pass when their fan-out reached the phone
//! (their receipts are not the point of this run). Empty answers are accepted.

use tonic::transport::Channel;

use wamux::proto::v1 as pb;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;
use wamux_tools::delivery::{judge_reached, judge_send, judge_send_keeping_id};
use wamux_tools::media_kit::{media_chunks, png_bytes};
use wamux_tools::report::Report;
use wamux_types::relay_jid;

use crate::E2eCtx;

/// Send the text, then act on it. Returns the text's key for the final revoke
/// whenever the send went out, delivered or not: it is in the chat either way.
pub async fn run_sends(
    messaging: &mut MessagingServiceClient<Channel>,
    report: &mut Report,
    ctx: &E2eCtx,
) -> Option<pb::MessageKey> {
    let (key, delivered) = send_text(messaging, report, ctx).await;
    let presence = messaging
        .send_presence(pb::SendPresenceRequest {
            account: Some(ctx.acct.clone()),
            chat: Some(jid_of(&ctx.dest)),
            state: pb::PresenceState::Composing as i32,
        })
        .await;
    report.accepted_rpc("Messaging.SendPresence(composing)", presence);
    if let Some(key) = key.as_ref().filter(|_| delivered) {
        react_and_edit(messaging, report, ctx, key).await;
    }
    send_image(messaging, report, ctx).await;
    let read = messaging
        .mark_read(pb::MarkReadRequest {
            account: Some(ctx.acct.clone()),
            chat: Some(jid_of(&ctx.dest)),
            message_ids: vec![],
            sender: None,
        })
        .await;
    report.accepted_rpc("Messaging.MarkRead", read);
    key
}

fn jid_of(value: &str) -> pb::Jid {
    pb::Jid {
        value: value.to_string(),
    }
}

async fn send_text(
    messaging: &mut MessagingServiceClient<Channel>,
    report: &mut Report,
    ctx: &E2eCtx,
) -> (Option<pb::MessageKey>, bool) {
    let sent = messaging
        .send_text(pb::SendTextRequest {
            account: Some(ctx.acct.clone()),
            to: Some(jid_of(&ctx.dest)),
            text: "wamux e2e_all: texto".to_string(),
            ..Default::default()
        })
        .await
        .map(tonic::Response::into_inner);
    let (id, delivered) =
        judge_send_keeping_id(report, "Messaging.SendText", sent, &ctx.tap, ctx.window).await;
    let key = id.map(|id| pb::MessageKey {
        chat: relay_jid(ctx.dest.clone()),
        id,
        from_me: true,
        participant: None,
    });
    (key, delivered)
}

async fn react_and_edit(
    messaging: &mut MessagingServiceClient<Channel>,
    report: &mut Report,
    ctx: &E2eCtx,
    key: &pb::MessageKey,
) {
    let reacted = messaging
        .send_reaction(pb::SendReactionRequest {
            account: Some(ctx.acct.clone()),
            target: Some(key.clone()),
            emoji: "\u{1F44D}".to_string(),
        })
        .await
        .map(tonic::Response::into_inner);
    judge_reached(report, "Messaging.SendReaction", reacted);
    let edited = messaging
        .edit_message(pb::EditMessageRequest {
            account: Some(ctx.acct.clone()),
            target: Some(key.clone()),
            new_text: "wamux e2e_all: texto (editado)".to_string(),
        })
        .await
        .map(tonic::Response::into_inner);
    judge_reached(report, "Messaging.EditMessage", edited);
}

async fn send_image(
    messaging: &mut MessagingServiceClient<Channel>,
    report: &mut Report,
    ctx: &E2eCtx,
) {
    let image = match png_bytes(500, 250) {
        Ok(bytes) => bytes,
        Err(e) => return report.fail("Messaging.SendMedia(image)", e.to_string()),
    };
    let header = pb::SendMediaHeader {
        account: Some(ctx.acct.clone()),
        to: Some(jid_of(&ctx.dest)),
        mime_type: "image/png".to_string(),
        caption: "wamux e2e_all: imagem".to_string(),
        media_type: pb::MediaType::Image as i32,
        filename: "e2e.png".to_string(),
        ..Default::default()
    };
    let sent = messaging
        .send_media(tokio_stream::iter(media_chunks(header, &image)))
        .await
        .map(tonic::Response::into_inner);
    judge_send(
        report,
        "Messaging.SendMedia(image)",
        sent,
        &ctx.tap,
        ctx.window,
    )
    .await;
}

/// Revoke our own text last (reversible: it is our message).
pub async fn revoke_text(
    messaging: &mut MessagingServiceClient<Channel>,
    report: &mut Report,
    ctx: &E2eCtx,
    key: Option<pb::MessageKey>,
) {
    let Some(key) = key else {
        return;
    };
    let revoked = messaging
        .delete_message(pb::DeleteMessageRequest {
            account: Some(ctx.acct.clone()),
            target: Some(key),
            for_everyone: true,
        })
        .await
        .map(tonic::Response::into_inner);
    judge_reached(report, "Messaging.DeleteMessage(revoke)", revoked);
}
