//! Validate sending VIDEO, AUDIO, STICKER and PTT (voice note) over the socket.
//! Video/audio are downloaded by this client and streamed inline (the core never
//! fetches URLs); sticker is an inline generated 512x512 WebP; ptt streams an
//! OGG/Opus file (WhatsApp only renders voice notes for OGG/Opus).
//!
//! Usage: send_types [socket_path] [target_number] [all|video|audio|sticker|ptt]
//!   defaults: /tmp/wamux.sock 5511999999999 all   ("all" excludes ptt: it needs a file)
//! Env: WAMUX_REF       account external_ref (default "pair-socket")
//!      WAMUX_PTT_FILE  OGG/Opus path for the ptt kind (default /tmp/wamux-ptt-test.ogg)
//!      WAMUX_PTT_SECS  voice note duration shown in the bubble (default 3)
//! Requires the daemon running with the account paired. The core relays to the
//! JID verbatim (no routing); this client targets @c.us to dodge the PN->LID upgrade.

use std::io::{Cursor, Read};
use std::time::Duration;

use hyper_util::rt::TokioIo;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;

const DEFAULT_REF: &str = "pair-socket";
const VIDEO_URL: &str = "https://www.w3schools.com/html/mov_bbb.mp4";
// WhatsApp does not process OGG/Vorbis; use MP3 (audio/mpeg) for an audio file.
const AUDIO_URL: &str = "https://www.w3schools.com/html/horse.mp3";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/wamux.sock".to_string());
    let number = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "5511999999999".to_string());
    let target = format!("{number}@c.us");

    let channel = connect_uds(socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut messaging = MessagingServiceClient::new(channel);
    let acct = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(
            std::env::var("WAMUX_REF").unwrap_or_else(|_| DEFAULT_REF.to_string()),
        )),
    };

    account
        .connect_account(pb::ConnectAccountRequest {
            account: Some(acct.clone()),
            backfill_history: false,
        })
        .await?;
    for _ in 0..100 {
        if let Ok(r) = account.get_account_status(acct.clone()).await
            && r.into_inner().state == pb::ConnectionState::Connected as i32
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let kinds = std::env::args().nth(3).unwrap_or_else(|| "all".to_string());
    let want = |k: &str| kinds == "all" || kinds == k;
    println!("connected; sending [{kinds}] to {target}");

    if want("video") {
        report(
            "video",
            send_url(
                &mut messaging,
                &acct,
                &target,
                "video/mp4",
                "video",
                VIDEO_URL,
            )
            .await,
        );
    }
    if want("audio") {
        report(
            "audio",
            send_url(
                &mut messaging,
                &acct,
                &target,
                "audio/mpeg",
                "audio",
                AUDIO_URL,
            )
            .await,
        );
    }
    if want("sticker") {
        report(
            "sticker",
            send_inline(
                &mut messaging,
                &acct,
                &target,
                "image/webp",
                "sticker",
                "s.webp",
                sticker_webp()?,
                None,
            )
            .await,
        );
    }
    // Explicit-only ("all" skips it): needs an OGG/Opus file on disk.
    if kinds == "ptt" {
        let path = std::env::var("WAMUX_PTT_FILE")
            .unwrap_or_else(|_| "/tmp/wamux-ptt-test.ogg".to_string());
        let secs: u32 = std::env::var("WAMUX_PTT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3);
        let data = std::fs::read(&path).map_err(|e| anyhow::anyhow!("read {path}: {e}"))?;
        report(
            "ptt",
            send_inline(
                &mut messaging,
                &acct,
                &target,
                "audio/ogg; codecs=opus",
                "audio",
                "",
                data,
                Some(secs),
            )
            .await,
        );
    }

    Ok(())
}

fn report(kind: &str, r: Result<String, String>) {
    match r {
        Ok(id) => println!("✅ {kind} sent id={id}"),
        Err(e) => println!("❌ {kind} failed: {e}"),
    }
}

/// The core no longer fetches URLs; the client (acting as the edge) downloads
/// the bytes itself and streams them inline.
async fn send_url(
    m: &mut MessagingServiceClient<Channel>,
    acct: &pb::AccountRef,
    target: &str,
    mime: &str,
    media_type: &str,
    url: &str,
) -> Result<String, String> {
    let url = url.to_string();
    let data = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
        let resp = ureq::get(&url).call().map_err(|e| e.to_string())?;
        let mut buf = Vec::new();
        resp.into_reader()
            .read_to_end(&mut buf)
            .map_err(|e| e.to_string())?;
        Ok(buf)
    })
    .await
    .map_err(|e| e.to_string())??;
    send_inline(m, acct, target, mime, media_type, "", data, None).await
}

#[allow(clippy::too_many_arguments)] // flat wire-field list; a builder would obscure the proto shape
async fn send_inline(
    m: &mut MessagingServiceClient<Channel>,
    acct: &pb::AccountRef,
    target: &str,
    mime: &str,
    media_type: &str,
    filename: &str,
    data: Vec<u8>,
    ptt_seconds: Option<u32>,
) -> Result<String, String> {
    let header = pb::SendMediaChunk {
        part: Some(pb::send_media_chunk::Part::Header(pb::SendMediaHeader {
            account: Some(acct.clone()),
            to: Some(pb::Jid {
                value: target.to_string(),
            }),
            mime_type: mime.to_string(),
            caption: String::new(),
            mentions: vec![],
            quote: None,
            media_type: media_type.to_string(),
            filename: filename.to_string(),
            ptt: ptt_seconds.is_some(),
            seconds: ptt_seconds.unwrap_or(0),
            waveform: vec![],
            ephemeral_seconds: 0,
        })),
    };
    let mut chunks = vec![header];
    for c in data.chunks(64 * 1024) {
        chunks.push(pb::SendMediaChunk {
            part: Some(pb::send_media_chunk::Part::Chunk(c.to_vec())),
        });
    }
    let r = m
        .send_media(tokio_stream::iter(chunks))
        .await
        .map_err(|e| e.message().to_string())?;
    Ok(r.into_inner().key.map(|k| k.id).unwrap_or_default())
}

fn sticker_webp() -> anyhow::Result<Vec<u8>> {
    let mut img = image::RgbaImage::new(512, 512);
    for (x, y, p) in img.enumerate_pixels_mut() {
        *p = image::Rgba([(x / 2) as u8, (y / 2) as u8, 180, 255]);
    }
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::WebP)?;
    Ok(buf)
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
