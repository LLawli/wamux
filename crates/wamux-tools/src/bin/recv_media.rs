//! Wait for an inbound media message on the already-paired account and validate
//! MediaService.DownloadMedia. No pairing; assumes WAMUX_REF is paired.
//!
//! Env: WAMUX_REF (required), WAMUX_SOCKET_PATH.
//! Usage: recv_media [seconds]   (default 300)

use std::process::ExitCode;
use std::time::Duration;

use tonic::transport::Channel;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::proto::v1::media_service_client::MediaServiceClient;
use wamux_tools::live_env::{account_ref_from, process_env, socket_path_from};
use wamux_tools::report::Report;
use wamux_tools::socket_client::{account_ref, connect_uds, wait_connected};

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let external_ref: String = account_ref_from(&process_env)?;
    let socket: String = socket_path_from(&process_env)?;
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    let channel = connect_uds(&socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut media = MediaServiceClient::new(channel.clone());
    let mut events = EventServiceClient::new(channel);
    let acct = account_ref(&external_ref);

    wait_connected(&mut account, &acct, Duration::from_secs(30)).await?;
    println!("connected; waiting up to {secs}s for inbound media ...");
    let sub = pb::SubscribeRequest {
        selector: Some(pb::subscribe_request::Selector::Account(acct.clone())),
        replay_from_ring: 0,
    };
    let stream = events.subscribe_events(sub).await?.into_inner();

    let mut report = Report::new();
    match wait_for_media(stream, secs).await {
        Some(descriptor) => download(&mut media, &mut report, acct, descriptor).await,
        None => report.fail(
            "Event.InboundMedia",
            format!("no inbound media received within {secs}s"),
        ),
    }
    Ok(report.finish())
}

async fn wait_for_media(
    mut ev: tonic::Streaming<pb::EventEnvelope>,
    secs: u64,
) -> Option<pb::MediaDescriptor> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(5), ev.message()).await {
            Ok(Ok(Some(env))) => {
                let Some(pb::event_envelope::Event::Message(m)) = env.event else {
                    continue;
                };
                println!(
                    "[recv] from={} text={:?} media={:?}",
                    m.sender,
                    m.text,
                    m.media.as_ref().map(|d| &d.media_type)
                );
                if m.media.is_some() {
                    return m.media;
                }
            }
            Ok(Ok(None)) | Ok(Err(_)) => break,
            Err(_) => {}
        }
    }
    None
}

async fn download(
    media: &mut MediaServiceClient<Channel>,
    report: &mut Report,
    acct: pb::AccountRef,
    descriptor: pb::MediaDescriptor,
) {
    let (mime, mtype) = (descriptor.mime_type.clone(), descriptor.media_type.clone());
    let request = pb::DownloadMediaRequest {
        account: Some(acct),
        descriptor: Some(descriptor),
    };
    let mut stream = match media.download_media(request).await {
        Ok(response) => response.into_inner(),
        Err(status) => return report.fail("Media.DownloadMedia", status.to_string()),
    };
    let mut bytes: Vec<u8> = Vec::new();
    loop {
        match stream.message().await {
            Ok(Some(chunk)) => {
                if let Some(pb::media_chunk::Part::Chunk(piece)) = chunk.part {
                    bytes.extend_from_slice(&piece);
                }
            }
            Ok(None) => break,
            Err(status) => return report.fail("Media.DownloadMedia", status.to_string()),
        }
    }
    let path = format!("/tmp/wamux-download-{mtype}.bin");
    if let Err(e) = std::fs::write(&path, &bytes) {
        eprintln!("cannot save {path}: {e}");
    }
    report.verify(
        "Media.DownloadMedia",
        !bytes.is_empty(),
        format!(
            "{} bytes (mime={mime}, type={mtype}) -> {path}",
            bytes.len()
        ),
    );
}
