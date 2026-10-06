//! Validate the history-sync backfill path end-to-end over the socket.
//!
//! Flow (mirrors how a CRM would backfill a chat):
//!   1. ConnectAccount with backfill_history=true. This is REQUIRED: the skip
//!      gate in the library drops ALL history sync (incl. on-demand answers),
//!      so without it FetchMessageHistory results never arrive.
//!   2. SubscribeEvents; capture a real anchor (chat + oldest msg key + ts) from
//!      the first live inbound message (or use the chat jid passed as arg 1).
//!   3. FetchMessageHistory(anchor, count) -> session_id.
//!   4. Watch for the HistorySyncEvent whose session_id matches; decode the raw
//!      `wa.HistorySync` and count conversations/messages.
//!
//! Env: WAMUX_REF (required), WAMUX_SOCKET_PATH.
//! Usage: backfill [chat_jid] [count] [watch_secs]   (defaults: live capture 50 40)

use std::process::ExitCode;
use std::time::Duration;

use whatsapp_rust::buffa::Message as _;
use whatsapp_rust::waproto::whatsapp as wa;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;
use wamux_tools::live_env::{account_ref_from, jid_text_of, process_env, socket_path_from};
use wamux_tools::report::Report;
use wamux_tools::socket_client::{account_ref, connect_uds, wait_connected_with};

struct Anchor {
    chat: String,
    msg_id: String,
    from_me: bool,
    ts_ms: i64,
}

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let external_ref: String = account_ref_from(&process_env)?;
    let socket: String = socket_path_from(&process_env)?;
    let chat_arg = std::env::args().nth(1).filter(|s| !s.is_empty());
    let count: i32 = arg(2, "50").parse().unwrap_or(50);
    let watch_secs: u64 = arg(3, "40").parse().unwrap_or(40);

    let channel = connect_uds(&socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut events = EventServiceClient::new(channel.clone());
    let mut messaging = MessagingServiceClient::new(channel);
    let acct = account_ref(&external_ref);

    println!("connecting '{external_ref}' with backfill_history=true ...");
    wait_connected_with(&mut account, &acct, Duration::from_secs(30), true).await?;
    println!("connected");

    let mut stream = events
        .subscribe_events(pb::SubscribeRequest {
            selector: Some(pb::subscribe_request::Selector::Account(acct.clone())),
            // Replay buffered events so we can anchor on a recently-seen message
            // even if the live stream is momentarily quiet.
            replay_from_ring: 256,
        })
        .await?
        .into_inner();

    let mut report = Report::new();
    let Some(anchor) = find_anchor(&mut stream, chat_arg).await else {
        report.fail(
            "Anchor",
            "no inbound message arrived in 30s; pass a chat jid as the first argument instead",
        );
        return Ok(report.finish());
    };
    let request = pb::FetchMessageHistoryRequest {
        account: Some(acct),
        chat: Some(pb::Jid { value: anchor.chat }),
        oldest_msg_id: anchor.msg_id,
        oldest_msg_from_me: anchor.from_me,
        oldest_msg_timestamp_ms: anchor.ts_ms,
        count,
    };
    let session = match messaging.fetch_message_history(request).await {
        Ok(resp) => resp.into_inner().session_id,
        Err(status) => {
            report.fail("Messaging.FetchMessageHistory", status.to_string());
            return Ok(report.finish());
        }
    };
    report.verify(
        "Messaging.FetchMessageHistory",
        !session.is_empty(),
        format!("session_id={session}"),
    );
    println!("watching {watch_secs}s for the answer ...\n");
    let got = watch_history(&mut stream, &session, watch_secs).await;
    report.verify(
        "HistorySyncEvent answers the session",
        got,
        format!("session_id={session}"),
    );
    if !got {
        println!(
            "The phone may not have older messages for this chat, or took >{watch_secs}s.\n\
             Retry with a busier chat, or re-pair a fresh account to see InitialBootstrap."
        );
    }
    Ok(report.finish())
}

/// The chat given on the command line (empty anchor), else the first live
/// inbound message within 30s.
async fn find_anchor(
    stream: &mut tonic::Streaming<pb::EventEnvelope>,
    chat_arg: Option<String>,
) -> Option<Anchor> {
    if let Some(chat) = chat_arg {
        println!("using chat from arg: {chat} (empty anchor)");
        return Some(Anchor {
            chat,
            msg_id: String::new(),
            from_me: false,
            ts_ms: 0,
        });
    }
    println!("waiting up to 30s for a live inbound message to anchor on ...");
    let anchor = capture_anchor(stream, Duration::from_secs(30)).await?;
    println!(
        "anchor: chat={} msg_id={} from_me={} ts_ms={}",
        anchor.chat, anchor.msg_id, anchor.from_me, anchor.ts_ms
    );
    Some(anchor)
}

/// Whether a HistorySyncEvent carrying `session` arrives within `watch_secs`.
async fn watch_history(
    stream: &mut tonic::Streaming<pb::EventEnvelope>,
    session: &str,
    watch_secs: u64,
) -> bool {
    let mut got = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(watch_secs);
    while let Some(env) = next_until(stream, deadline).await {
        let Some(pb::event_envelope::Event::HistorySync(h)) = env.event else {
            continue;
        };
        let (convs, msgs) = decode_counts(&h.raw);
        let matches = h.session_id.as_deref() == Some(session);
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
        got |= matches;
    }
    got
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
                chat: jid_text_of(&m.chat).to_string(),
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
    match wa::HistorySync::decode_from_slice(raw) {
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
