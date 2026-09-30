//! Wait for an inbound media message on the already-paired account and validate
//! MediaService.DownloadMedia. No pairing; assumes "pair-socket" is paired.
//!
//! Usage: recv_media [socket_path] [seconds]   (defaults /tmp/wamux.sock 300)

use std::time::Duration;

use hyper_util::rt::TokioIo;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::proto::v1::media_service_client::MediaServiceClient;

const EXTERNAL_REF: &str = "pair-socket";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/wamux.sock".to_string());
    let secs: u64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    let channel = connect_uds(socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut media = MediaServiceClient::new(channel.clone());
    let mut events = EventServiceClient::new(channel);
    let acct = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(EXTERNAL_REF.to_string())),
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
    println!("connected; waiting up to {secs}s for inbound media ...");

    let sub = pb::SubscribeRequest {
        selector: Some(pb::subscribe_request::Selector::Account(acct.clone())),
        replay_from_ring: 0,
    };
    let mut ev = events.subscribe_events(sub).await?.into_inner();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    let mut descriptor: Option<pb::MediaDescriptor> = None;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(5), ev.message()).await {
            Ok(Ok(Some(env))) => {
                if let Some(pb::event_envelope::Event::Message(m)) = env.event {
                    println!(
                        "[recv] from={} text={:?} media={:?}",
                        m.sender,
                        m.text,
                        m.media.as_ref().map(|d| &d.media_type)
                    );
                    if let Some(d) = m.media {
                        descriptor = Some(d);
                        break;
                    }
                }
            }
            Ok(Ok(None)) | Ok(Err(_)) => break,
            Err(_) => {}
        }
    }

    match descriptor {
        Some(d) => {
            let (mime, mtype) = (d.mime_type.clone(), d.media_type.clone());
            match media
                .download_media(pb::DownloadMediaRequest {
                    account: Some(acct),
                    descriptor: Some(d),
                })
                .await
            {
                Ok(s) => {
                    let mut s = s.into_inner();
                    let mut bytes = Vec::new();
                    while let Ok(Some(c)) = s.message().await {
                        if let Some(pb::media_chunk::Part::Chunk(b)) = c.part {
                            bytes.extend_from_slice(&b);
                        }
                    }
                    let path = format!("/tmp/wamux-download-{mtype}.bin");
                    let _ = std::fs::write(&path, &bytes);
                    println!(
                        "✅ DownloadMedia OK: {} bytes (mime={mime}, type={mtype}) -> {path}",
                        bytes.len()
                    );
                }
                Err(e) => println!("❌ DownloadMedia: {}", e.message()),
            }
        }
        None => println!("❌ no inbound media received within {secs}s"),
    }
    Ok(())
}

async fn connect_uds(path: String) -> anyhow::Result<Channel> {
    let channel = Endpoint::try_from("http://[::1]:50051")?
        .connect_with_connector(service_fn(move |_: Uri| {
            let path = path.clone();
            async move {
                Ok::<_, std::io::Error>(TokioIo::new(tokio::net::UnixStream::connect(path).await?))
            }
        }))
        .await?;
    Ok(channel)
}
