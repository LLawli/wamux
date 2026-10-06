//! Live validation for issue #20: the read receipt, and the app-state sync that
//! used to wear its name.
//!
//! SAFETY: the destination comes from `WAMUX_LIVE_DEST` (one JID) and the bin
//! refuses to run without it, the same guard `stress_live`, `poll_live` and
//! `sticker_live` use. It also refuses to target its own number.
//!
//! Usage: WAMUX_LIVE_DEST=<jid> read_receipt_live [seconds]   (default 240)
//! Env: WAMUX_REF            account external_ref (required)
//!      WAMUX_LIVE_DEST      the one chat to send to (required)
//!      WAMUX_DELIVERY_SECS  how long the prompt waits for its `delivered` receipt
//!
//! What it does, in order:
//!   1. sends a text, so the chat surfaces on the other handset;
//!   2. waits for a message FROM that chat -- reply from the other phone;
//!   3. MarkRead on that message id: the receipt;
//!   4. MarkChatRead: the app-state sync.
//!
//! Where to look, which is NOT the same device for the two:
//!   - the receipt shows up on the OTHER phone, as blue ticks on the message it
//!     sent. Nothing about it is visible from this side, ever.
//!   - the sync shows up on THIS account's own linked devices, as the chat
//!     losing its unread badge. The other phone never sees it.
//!
//! Neither call answers with anything but `Empty`. That silence is the whole
//! reason #20 went unnoticed, and it is still true after the fix: a call that
//! succeeds and a receipt that arrives are different claims. So both are
//! recorded as `accepted`, never `pass`; only the prompt's delivery and the
//! reply's arrival are asserted values.

use std::process::ExitCode;
use std::time::Duration;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;
use wamux_tools::delivery::{EventTap, judge_send};
use wamux_tools::live_env::{
    account_ref_from, delivery_window_from, jid_text_of, live_dest_from, process_env,
    refuse_own_number, socket_path_from, user_of,
};
use wamux_tools::report::Report;
use wamux_tools::socket_client::{account_ref, connect_uds, wait_connected};

const PROMPT: &str = "wamux #20: responda esta mensagem para eu marcar como lida";

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let external_ref: String = account_ref_from(&process_env)?;
    let socket: String = socket_path_from(&process_env)?;
    let chat: String = live_dest_from(&process_env)?;
    let window: Duration = delivery_window_from(&process_env)?;
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(240);

    let channel = connect_uds(&socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut messaging = MessagingServiceClient::new(channel.clone());
    let mut events = EventServiceClient::new(channel);
    let acct = account_ref(&external_ref);

    let own = wait_connected(&mut account, &acct, Duration::from_secs(30)).await?;
    refuse_own_number(&own, &chat)?;
    println!("connected as {own}");

    // Subscribe BEFORE the prompt: a fast reply would otherwise land before the
    // stream attaches and there would be no id to acknowledge.
    let stream = events
        .subscribe_events(pb::SubscribeRequest {
            selector: Some(pb::subscribe_request::Selector::Account(acct.clone())),
            replay_from_ring: 0,
        })
        .await?
        .into_inner();
    let tap = EventTap::spawn(stream);
    let mut report = Report::new();

    let sent = messaging
        .send_text(pb::SendTextRequest {
            account: Some(acct.clone()),
            to: Some(pb::Jid {
                value: chat.clone(),
            }),
            text: PROMPT.to_string(),
            ..Default::default()
        })
        .await
        .map(tonic::Response::into_inner);
    let Some(prompt_id) = judge_send(
        &mut report,
        "Messaging.SendText(prompt)",
        sent,
        &tap,
        window,
    )
    .await
    else {
        return Ok(report.finish());
    };
    println!("prompt sent to {chat} (id {prompt_id}); reply from the other phone");

    let reply = wait_for_reply(&tap, &chat, secs).await;
    report.verify(
        "Event.InboundMessage reply",
        reply.is_some(),
        format!("a reply from {chat} within {secs}s"),
    );
    let Some(inbound) = reply else {
        return Ok(report.finish());
    };
    let message_id = inbound.key.clone().map(|key| key.id).unwrap_or_default();
    println!(
        "[reply] from {} id={message_id}",
        jid_text_of(&inbound.sender)
    );

    // A receipt names whose messages are being acknowledged. In a DM the chat IS
    // the author, so the field stays absent; a group needs it (issue #20).
    let sender = jid_text_of(&inbound.chat)
        .ends_with("@g.us")
        .then(|| inbound.sender.clone())
        .flatten();
    let read = messaging
        .mark_read(pb::MarkReadRequest {
            account: Some(acct.clone()),
            chat: inbound.chat.clone(),
            message_ids: vec![message_id],
            sender,
        })
        .await;
    report.accepted_rpc("Messaging.MarkRead (receipt)", read);
    println!("look at the OTHER phone: blue ticks on the message it sent");

    tokio::time::sleep(Duration::from_secs(3)).await;
    let synced = messaging
        .mark_chat_read(pb::MarkReadRequest {
            account: Some(acct.clone()),
            chat: inbound.chat.clone(),
            ..Default::default()
        })
        .await;
    report.accepted_rpc("Messaging.MarkChatRead (app-state)", synced);
    println!("look at THIS account's own app: the chat drops its unread badge");
    Ok(report.finish())
}

/// The first inbound message in this chat that this account did not send.
async fn wait_for_reply(tap: &EventTap, chat: &str, secs: u64) -> Option<pb::InboundMessage> {
    let wanted = user_of(chat);
    let found = tap
        .wait_for(Duration::from_secs(secs), |envelope| {
            reply_from(envelope, wanted).is_some()
        })
        .await?;
    reply_from(&found, wanted).cloned()
}

fn reply_from<'a>(envelope: &'a pb::EventEnvelope, wanted: &str) -> Option<&'a pb::InboundMessage> {
    let Some(pb::event_envelope::Event::Message(inbound)) = &envelope.event else {
        return None;
    };
    let from_me = inbound.key.as_ref().is_some_and(|key| key.from_me);
    // Match on the user part: the chat may arrive `@lid` while the prompt
    // went to a phone jid, and either form is the same conversation.
    let same_chat = user_of(jid_text_of(&inbound.chat)) == wanted
        || user_of(jid_text_of(&inbound.sender_alt)) == wanted
        || user_of(jid_text_of(&inbound.sender)) == wanted;
    (!from_me && same_chat && !inbound.text.is_empty()).then_some(inbound)
}
