//! Validate the history-sync backfill path end-to-end over the socket.
//!
//! Flow (mirrors how a CRM would backfill a chat):
//!   1. ConnectAccount with backfill_history=true. This is REQUIRED: the skip
//!      gate in the library drops ALL history sync (incl. on-demand answers),
//!      so without it FetchMessageHistory results never arrive.
//!   2. SubscribeEvents; capture a real anchor (chat + oldest msg key + ts) from
//!      the first live inbound message (or use the chat jid passed as arg 3).
//!   3. FetchMessageHistory(anchor, count) -> session_id.
//!   4. Watch for the HistorySyncEvent whose session_id matches; decode the raw
//!      `wa.HistorySync` and count conversations/messages.
//!
//! Usage: backfill [socket] [external_ref] [chat_jid] [count]
//!   defaults: /tmp/wamux.sock  pair-socket  (capture from live msg)  50

use std::time::Duration;

use hyper_util::rt::TokioIo;
use prost014::Message as _;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;
use whatsapp_rust::waproto::whatsapp as wa;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;

struct Anchor {
    chat: String,
    msg_id: String,
    from_me: bool,
    ts_ms: i64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket = arg(1, "/tmp/wamux.sock");
    let external_ref = arg(2, "pair-socket");
    let chat_arg = std::env::args().nth(3).filter(|s| !s.is_empty());
    let count: i32 = arg(4, "50").parse().unwrap_or(50);
    let watch_secs: u64 = arg(5, "40").parse().unwrap_or(40);

    let channel = connect_uds(socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut events = EventServiceClient::new(channel.clone());
    let mut messaging = MessagingServiceClient::new(channel);

    let acct = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(external_ref.clone())),
    };

    println!("connecting '{external_ref}' with backfill_history=true ...");
    account
        .connect_account(pb::ConnectAccountRequest {
            account: Some(acct.clone()),
            backfill_history: true,
        })
        .await?;
    anyhow::ensure!(
        wait_connected(&mut account, &acct).await,
        "account did not reach CONNECTED"
    );
    println!("connected ✅");

    let mut stream = events
        .subscribe_events(pb::SubscribeRequest {
            selector: Some(pb::subscribe_request::Selector::Account(acct.clone())),
            // Replay buffered events so we can anchor on a recently-seen message
            // even if the live stream is momentarily quiet.
            replay_from_ring: 256,
        })
        .await?
        .into_inner();

    // --- get an anchor ---
    let anchor = if let Some(chat) = chat_arg {
        println!("using chat from arg: {chat} (empty anchor)");
        Anchor {
            chat,
            msg_id: String::new(),
            from_me: false,
            ts_ms: 0,
        }
    } else {
        println!("waiting up to 30s for a live inbound message to anchor on ...");
        match capture_anchor(&mut stream, Duration::from_secs(30)).await {
            Some(a) => {
                println!(
                    "anchor: chat={} msg_id={} from_me={} ts_ms={}",
                    a.chat, a.msg_id, a.from_me, a.ts_ms
                );
                a
            }
            None => {
                println!("no inbound message arrived; pass a chat jid as arg 3 instead.");
                return Ok(());
            }
        }
    };

    // --- request on-demand history ---
    let resp = messaging
        .fetch_message_history(pb::FetchMessageHistoryRequest {
            account: Some(acct.clone()),
            chat: Some(pb::Jid {
                value: anchor.chat.clone(),
            }),
            oldest_msg_id: anchor.msg_id.clone(),
            oldest_msg_from_me: anchor.from_me,
            oldest_msg_timestamp_ms: anchor.ts_ms,
            count,
        })
        .await?
        .into_inner();
    let session = resp.session_id;
    println!(
        "FetchMessageHistory -> session_id={session}\nwatching {watch_secs}s for the answer ...\n"
    );

    // --- watch for the matching HistorySyncEvent ---
    let mut got = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(watch_secs);
    while let Some(env) = next_until(&mut stream, deadline).await {
        if let Some(pb::event_envelope::Event::HistorySync(h)) = env.event {
            let (convs, msgs) = decode_counts(&h.raw);
            let matches = h.session_id.as_deref() == Some(session.as_str());
            println!(
                "[history] sync_type={} chunk={:?} progress={:?} session={:?}{} raw={}B convs={} msgs={}",
                h.sync_type,
                h.chunk_order,
                h.progress,
                h.session_id,
                if matches { " (MATCHES)" } else { "" },
                h.raw.len(),
                convs,
                msgs
            );
            if matches {
                got = true;
            }
        }
    }

    println!(
        "\n=== summary ===\nFetchMessageHistory answer received & matched: {}",
        if got { "YES ✅" } else { "NO" }
    );
    if !got {
        println!(
            "If NO: the phone may not have older messages for this chat, or took >40s.\n\
             Retry with a busier chat, or re-pair a fresh account to see InitialBootstrap."
        );
    }
    Ok(())
}

async fn wait_connected(
    account: &mut AccountServiceClient<Channel>,
    acct: &pb::AccountRef,
) -> bool {
    for _ in 0..100 {
        if let Ok(s) = account.get_account_status(acct.clone()).await
            && s.into_inner().state == pb::ConnectionState::Connected as i32
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    false
}

async fn capture_anchor(
    stream: &mut tonic::Streaming<pb::EventEnvelope>,
    window: Duration,
) -> Option<Anchor> {
    let deadline = tokio::time::Instant::now() + window;
    while let Some(env) = next_until(stream, deadline).await {
        if let Some(pb::event_envelope::Event::Message(m)) = env.event
            && let Some(key) = m.key
            && !key.id.is_empty()
        {
            return Some(Anchor {
                chat: m.chat,
                msg_id: key.id,
                from_me: key.from_me,
                ts_ms: m.timestamp,
            });
        }
    }
    None
}

/// Next event before `deadline`, or None when the window elapses / stream ends.
async fn next_until(
    stream: &mut tonic::Streaming<pb::EventEnvelope>,
    deadline: tokio::time::Instant,
) -> Option<pb::EventEnvelope> {
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    if remaining.is_zero() {
        return None;
    }
    match tokio::time::timeout(remaining, stream.message()).await {
        Ok(Ok(Some(env))) => Some(env),
        _ => None,
    }
}

fn decode_counts(raw: &[u8]) -> (usize, usize) {
    match wa::HistorySync::decode(raw) {
        Ok(hs) => {
            let convs = hs.conversations.len();
            let msgs = hs.conversations.iter().map(|c| c.messages.len()).sum();
            (convs, msgs)
        }
        Err(_) => (0, 0),
    }
}

fn arg(n: usize, default: &str) -> String {
    std::env::args()
        .nth(n)
        .unwrap_or_else(|| default.to_string())
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
