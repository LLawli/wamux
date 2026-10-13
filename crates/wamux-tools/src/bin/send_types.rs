//! Validate sending VIDEO, AUDIO, STICKER and PTT (voice note) over the socket.
//! Video/audio are downloaded by this client and streamed inline (the core never
//! fetches URLs); sticker is an inline generated 512x512 WebP; ptt streams an
//! OGG/Opus file (WhatsApp only renders voice notes for OGG/Opus).
//!
//! Usage: send_types [all|video|audio|sticker|ptt]   ("all" excludes ptt: it needs a file)
//! Env: WAMUX_REF       account external_ref (required)
//!      WAMUX_LIVE_DEST the one chat to send to (required, @s.whatsapp.net)
//!      WAMUX_DELIVERY_SECS  how long each send waits for its `delivered` receipt
//!      WAMUX_VIDEO_FILE  a local video sent instead of the sample URL's. It streams from
//!                   disk in chunks and is never held whole in memory, so it can be as
//!                   large as the daemon's media_max_bytes (the live check of the media
//!                   spool, #79)
//!      WAMUX_PTT_FILE  OGG/Opus path for the ptt kind (default /tmp/wamux-ptt-test.ogg)
//!      WAMUX_PTT_SECS  voice note duration shown in the bubble (default 3)
//! Requires the daemon running with the account paired. The core relays to the
//! JID verbatim (no routing); this client targets @s.whatsapp.net.
//!
//! NOT the legacy server, which is what this bin used to send and what its
//! comment used to recommend. Issue #4 measured the cost: the legacy spelling
//! parses as `Server::Legacy`, the send path stops treating the recipient as a
//! phone user, no session is built, and the stanza ships encrypted for nobody
//! -- while still answering with a real message id and a server ack. A send
//! passes here only when the fan-out reached the phone and `delivered` came.

use std::process::ExitCode;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;
use wamux_tools::delivery::{EventTap, judge_send};
use wamux_tools::live_env::{
    account_ref_from, delivery_window_from, live_dest_from, process_env, refuse_own_number,
    socket_path_from,
};
use wamux_tools::media_kit::{
    MEDIA_CHUNK_BYTES, fetch_url_capped, media_chunks, webp_sticker_bytes,
};
use wamux_tools::report::Report;
use wamux_tools::socket_client::{account_ref, connect_uds, wait_connected};

const VIDEO_URL: &str = "https://www.w3schools.com/html/mov_bbb.mp4";
// WhatsApp does not process OGG/Vorbis; use MP3 (audio/mpeg) for an audio file.
const AUDIO_URL: &str = "https://www.w3schools.com/html/horse.mp3";
/// Largest sample file the client downloads before streaming it inline.
const DOWNLOAD_CAP_BYTES: usize = 32 * 1024 * 1024;

/// One media message: the wire fields that differ between the kinds.
struct InlineMedia {
    mime: &'static str,
    media_type: pb::MediaType,
    filename: &'static str,
    body: Body,
    ptt_seconds: Option<u32>,
}

/// Where the bytes come from: memory, or a file streamed in chunks.
enum Body {
    Bytes(Vec<u8>),
    File(String),
}

/// Who sends, to whom, and how long a send waits for its receipt.
struct SendCtx {
    acct: pb::AccountRef,
    target: String,
    tap: EventTap,
    window: Duration,
}

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let external_ref: String = account_ref_from(&process_env)?;
    let socket: String = socket_path_from(&process_env)?;
    let target: String = live_dest_from(&process_env)?;
    let window: Duration = delivery_window_from(&process_env)?;
    let kinds = std::env::args().nth(1).unwrap_or_else(|| "all".to_string());

    let channel = connect_uds(&socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut events = EventServiceClient::new(channel.clone());
    let mut messaging = MessagingServiceClient::new(channel);
    let acct = account_ref(&external_ref);

    let own = wait_connected(&mut account, &acct, Duration::from_secs(30)).await?;
    refuse_own_number(&own, &target)?;
    println!("connected as {own}; sending [{kinds}] to {target}");
    // Subscribe BEFORE the first send, or its receipt can land unseen.
    let stream = events
        .subscribe_events(pb::SubscribeRequest {
            selector: Some(pb::subscribe_request::Selector::Account(acct.clone())),
            replay_from_ring: 0,
        })
        .await?
        .into_inner();
    let ctx = SendCtx {
        acct,
        target,
        tap: EventTap::spawn(stream),
        window,
    };
    let mut report = Report::new();
    let want = |k: &str| kinds == "all" || kinds == k;

    if want("video") {
        let media = match process_env("WAMUX_VIDEO_FILE") {
            Some(path) => Ok(video_file(path)),
            None => download("video/mp4", pb::MediaType::Video, VIDEO_URL).await,
        };
        send_kind(&mut messaging, &mut report, &ctx, "video", media).await;
    }
    if want("audio") {
        let media = download("audio/mpeg", pb::MediaType::Audio, AUDIO_URL).await;
        send_kind(&mut messaging, &mut report, &ctx, "audio", media).await;
    }
    if want("sticker") {
        let media = webp_sticker_bytes()
            .map_err(|e| e.to_string())
            .map(|data| InlineMedia {
                mime: "image/webp",
                media_type: pb::MediaType::Sticker,
                filename: "s.webp",
                body: Body::Bytes(data),
                ptt_seconds: None,
            });
        send_kind(&mut messaging, &mut report, &ctx, "sticker", media).await;
    }
    // Explicit-only ("all" skips it): needs an OGG/Opus file on disk.
    if kinds == "ptt" {
        let media = voice_note().await;
        send_kind(&mut messaging, &mut report, &ctx, "ptt", media).await;
    }
    Ok(report.finish())
}

/// A media failure before the send (download, encode, file) is a failed check
/// for that kind, not a reason to stop the other kinds.
async fn send_kind(
    m: &mut MessagingServiceClient<Channel>,
    report: &mut Report,
    ctx: &SendCtx,
    kind: &str,
    media: Result<InlineMedia, String>,
) {
    let name = format!("Messaging.SendMedia({kind})");
    match media {
        Ok(media) => {
            let sent = send_inline(m, ctx, media).await;
            judge_send(report, &name, sent, &ctx.tap, ctx.window).await;
        }
        Err(why) => report.fail(&name, why),
    }
}

/// The core no longer fetches URLs; the client (acting as the edge) downloads
/// the bytes itself and streams them inline.
async fn download(
    mime: &'static str,
    media_type: pb::MediaType,
    url: &str,
) -> Result<InlineMedia, String> {
    let data = fetch_url_capped(url, DOWNLOAD_CAP_BYTES)
        .await
        .map_err(|e| e.to_string())?;
    Ok(InlineMedia {
        mime,
        media_type,
        filename: "",
        body: Body::Bytes(data),
        ptt_seconds: None,
    })
}

/// A local video, sent from disk: nothing reads it here until the stream does.
fn video_file(path: String) -> InlineMedia {
    InlineMedia {
        mime: "video/mp4",
        media_type: pb::MediaType::Video,
        filename: "",
        body: Body::File(path),
        ptt_seconds: None,
    }
}

async fn voice_note() -> Result<InlineMedia, String> {
    let path = process_env("WAMUX_PTT_FILE").unwrap_or_else(|| "/tmp/wamux-ptt-test.ogg".into());
    let secs: u32 = process_env("WAMUX_PTT_SECS")
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    // Async read so the runtime worker is not blocked on the file.
    let data = tokio::fs::read(&path)
        .await
        .map_err(|e| format!("read {path}: {e}"))?;
    Ok(InlineMedia {
        mime: "audio/ogg; codecs=opus",
        media_type: pb::MediaType::Audio,
        filename: "",
        body: Body::Bytes(data),
        ptt_seconds: Some(secs),
    })
}

async fn send_inline(
    m: &mut MessagingServiceClient<Channel>,
    ctx: &SendCtx,
    media: InlineMedia,
) -> Result<pb::SendResult, tonic::Status> {
    let header = pb::SendMediaHeader {
        account: Some(ctx.acct.clone()),
        to: Some(pb::Jid {
            value: ctx.target.clone(),
        }),
        mime_type: media.mime.to_string(),
        media_type: media.media_type as i32,
        filename: media.filename.to_string(),
        ptt: media.ptt_seconds.is_some(),
        seconds: media.ptt_seconds.unwrap_or(0),
        ..Default::default()
    };
    let frames = match media.body {
        Body::Bytes(data) => ReceiverStream::new(queued(media_chunks(header, &data))),
        Body::File(path) => file_chunks(header, path),
    };
    m.send_media(frames).await.map(tonic::Response::into_inner)
}

/// Frames already in hand, as a stream (the channel holds all of them).
fn queued(frames: Vec<pb::SendMediaChunk>) -> mpsc::Receiver<pb::SendMediaChunk> {
    let (tx, rx) = mpsc::channel(frames.len().max(1));
    for frame in frames {
        // Cannot fail: the channel was sized for every frame and rx is alive.
        let _ = tx.try_send(frame);
    }
    rx
}

/// The header, then the file read `MEDIA_CHUNK_BYTES` at a time. The channel is
/// small, so a slow daemon holds the reader back and memory stays one chunk wide.
fn file_chunks(header: pb::SendMediaHeader, path: String) -> ReceiverStream<pb::SendMediaChunk> {
    let (tx, rx) = mpsc::channel(4);
    tokio::spawn(async move {
        let head = pb::SendMediaChunk {
            part: Some(pb::send_media_chunk::Part::Header(header)),
        };
        if tx.send(head).await.is_err() {
            return;
        }
        let Ok(mut file) = tokio::fs::File::open(&path).await else {
            eprintln!("cannot open {path}");
            return;
        };
        loop {
            let mut piece = vec![0u8; MEDIA_CHUNK_BYTES];
            match file.read(&mut piece).await {
                Ok(0) | Err(_) => return,
                Ok(n) => piece.truncate(n),
            }
            let frame = pb::SendMediaChunk {
                part: Some(pb::send_media_chunk::Part::Chunk(piece)),
            };
            if tx.send(frame).await.is_err() {
                return;
            }
        }
    });
    ReceiverStream::new(rx)
}
