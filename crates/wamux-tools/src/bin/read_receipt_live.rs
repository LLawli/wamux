//! Live validation for issue #20: the read receipt, and the app-state sync that
//! used to wear its name.
//!
//! SAFETY: the destination comes from `WAMUX_LIVE_DEST` (one JID) and the bin
//! refuses to run without it, the same guard `stress_live`, `poll_live` and
//! `sticker_live` use. It also refuses to target its own number.
//!
//! Usage: WAMUX_LIVE_DEST=<jid> read_receipt_live [socket_path] [seconds]
//!   defaults: /tmp/wamux.sock 240
//! Env: WAMUX_REF  account external_ref (default "pair-socket")
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
//! succeeds and a receipt that arrives are different claims.

use std::time::Duration;

use hyper_util::rt::TokioIo;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;

const DEFAULT_REF: &str = "pair-socket";
const PROMPT: &str = "wamux #20: responda esta mensagem para eu marcar como lida";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket = arg(1, "/tmp/wamux.sock");
    let secs: u64 = arg(2, "240").parse().unwrap_or(240);
    let chat = std::env::var("WAMUX_LIVE_DEST").map_err(|_| {
        anyhow::anyhow!(
            "set WAMUX_LIVE_DEST to the destination JID (e.g. 5511999999999@s.whatsapp.net)"
        )
    })?;

    let channel = connect_uds(socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut messaging = MessagingServiceClient::new(channel.clone());
    let mut events = EventServiceClient::new(channel);
    let acct = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(
            std::env::var("WAMUX_REF").unwrap_or_else(|_| DEFAULT_REF.to_string()),
        )),
    };

    let own = wait_connected(&mut account, &acct).await?;
    if same_user(&own, &chat) {
        anyhow::bail!("WAMUX_LIVE_DEST ({chat}) is this account's own number; pick another");
    }
    println!("connected as {own}");

    // Subscribe BEFORE the prompt: a fast reply would otherwise land before the
    // stream attaches and there would be no id to acknowledge.
    let mut stream = events
        .subscribe_events(pb::SubscribeRequest {
            selector: Some(pb::subscribe_request::Selector::Account(acct.clone())),
            replay_from_ring: 0,
        })
        .await?
        .into_inner();

    let sent = messaging
        .send_text(pb::SendTextRequest {
            account: Some(acct.clone()),
            to: Some(pb::Jid {
                value: chat.clone(),
            }),
            text: PROMPT.to_string(),
            ..Default::default()
        })
        .await?
        .into_inner();
    println!(
        "prompt sent to {chat} (id {}); reply from the other phone",
        sent.key.map(|key| key.id).unwrap_or_default()
    );

    let Some(inbound) = wait_for_reply(&mut stream, &chat, secs).await else {
        println!("\u{274C} no reply arrived within {secs}s; nothing to acknowledge");
        return Ok(());
    };
    let message_id = inbound.key.clone().map(|key| key.id).unwrap_or_default();
    println!("[reply] from {} id={message_id}", inbound.sender);

    // A receipt names whose messages are being acknowledged. In a DM the chat IS
    // the author, so the field stays absent; a group needs it (issue #20).
    let sender = if inbound.chat.ends_with("@g.us") {
        Some(pb::Jid {
            value: inbound.sender.clone(),
        })
    } else {
        None
    };
    let read = messaging
        .mark_read(pb::MarkReadRequest {
            account: Some(acct.clone()),
            chat: Some(pb::Jid {
                value: inbound.chat.clone(),
            }),
            message_ids: vec![message_id.clone()],
            sender,
        })
        .await;
    report(
        "MarkRead (receipt)",
        read.map(|_| ()),
        "look at the OTHER phone: blue ticks on the message it sent",
    );

    tokio::time::sleep(Duration::from_secs(3)).await;
    let synced = messaging
        .mark_chat_read(pb::MarkReadRequest {
            account: Some(acct.clone()),
            chat: Some(pb::Jid {
                value: inbound.chat.clone(),
            }),
            ..Default::default()
        })
        .await;
    report(
        "MarkChatRead (app-state)",
        synced.map(|_| ()),
        "look at THIS account's own app: the chat drops its unread badge",
    );
    Ok(())
}

/// The first inbound message in this chat that this account did not send.
async fn wait_for_reply(
    stream: &mut tonic::Streaming<pb::EventEnvelope>,
    chat: &str,
    secs: u64,
) -> Option<pb::InboundMessage> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    let wanted = user_of(chat);
    while tokio::time::Instant::now() < deadline {
        let Ok(Ok(Some(envelope))) =
            tokio::time::timeout(Duration::from_secs(5), stream.message()).await
        else {
            continue;
        };
        let Some(pb::event_envelope::Event::Message(inbound)) = envelope.event else {
            continue;
        };
        let from_me = inbound.key.as_ref().is_some_and(|key| key.from_me);
        // Match on the user part: the chat may arrive `@lid` while the prompt
        // went to a phone jid, and either form is the same conversation.
        let same_chat = user_of(&inbound.chat) == wanted
            || user_of(&inbound.sender_alt) == wanted
            || user_of(&inbound.sender) == wanted;
        if !from_me && same_chat && !inbound.text.is_empty() {
            return Some(inbound);
        }
    }
    None
}

fn report(label: &str, result: Result<(), tonic::Status>, where_to_look: &str) {
    match result {
        Ok(()) => println!("\u{2705} {label}: accepted \u{2014} {where_to_look}"),
        Err(status) => println!("\u{274C} {label}: {}", status.message()),
    }
}

fn arg(index: usize, fallback: &str) -> String {
    std::env::args()
        .nth(index)
        .unwrap_or_else(|| fallback.to_string())
}

/// The user part of a jid, without the server or any device suffix.
fn user_of(jid: &str) -> String {
    jid.split('@')
        .next()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default()
        .to_string()
}

fn same_user(one: &str, other: &str) -> bool {
    user_of(one) == user_of(other)
}

async fn wait_connected(
    account: &mut AccountServiceClient<Channel>,
    acct: &pb::AccountRef,
) -> anyhow::Result<String> {
    account
        .connect_account(pb::ConnectAccountRequest {
            account: Some(acct.clone()),
            backfill_history: false,
        })
        .await?;
    for _ in 0..100 {
        let status = account.get_account_status(acct.clone()).await?.into_inner();
        if status.state == pb::ConnectionState::Connected as i32
            && let Some(jid) = status.jid
        {
            return Ok(jid.value);
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    anyhow::bail!("account did not reach Connected with a jid")
}

async fn connect_uds(path: String) -> anyhow::Result<Channel> {
    Ok(Endpoint::try_from("http://[::1]:50051")?
        .connect_with_connector(service_fn(move |_: Uri| {
            let path = path.clone();
            async move {
                Ok::<_, std::io::Error>(TokioIo::new(tokio::net::UnixStream::connect(path).await?))
            }
        }))
        .await?)
}
