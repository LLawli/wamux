//! Validate the two paths left untested: SetProfilePicture (square JPEG, with
//! restore) and DownloadMedia (from a received media descriptor). Drives the real
//! socket: pairs via QR (renders /tmp/wamux-qr.png + xdg-open), then runs both.
//!
//! Usage: validate1 [socket_path]   (default /tmp/wamux.sock)

use std::io::{Cursor, Read};
use std::time::Duration;

use hyper_util::rt::TokioIo;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::contact_service_client::ContactServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::proto::v1::media_service_client::MediaServiceClient;

const EXTERNAL_REF: &str = "pair-socket";
const QR_PNG: &str = "/tmp/wamux-qr.png";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/wamux.sock".to_string());
    let channel = connect_uds(socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut contacts = ContactServiceClient::new(channel.clone());
    let mut media = MediaServiceClient::new(channel.clone());
    let mut events = EventServiceClient::new(channel);
    let acct = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(EXTERNAL_REF.to_string())),
    };

    let _ = account
        .create_account(pb::CreateAccountRequest {
            external_ref: Some(EXTERNAL_REF.to_string()),
        })
        .await;

    // ---- Pair via QR ----
    println!("PairWithQr ... (scan the QR; it is written to {QR_PNG})");
    let mut stream = account
        .pair_with_qr(pb::PairWithQrRequest {
            account: Some(acct.clone()),
            backfill_history: false,
        })
        .await?
        .into_inner();
    let mut opened = false;
    let mut paired_jid = String::new();
    while let Some(update) = stream.message().await? {
        match update.event {
            Some(pb::pairing_update::Event::QrCode(code)) => {
                if write_qr_png(&code, QR_PNG).is_ok() {
                    println!("[qr] new QR -> {QR_PNG}");
                    if !opened {
                        let _ = std::process::Command::new("xdg-open").arg(QR_PNG).spawn();
                        opened = true;
                    }
                }
                println!("{}", ascii_qr(&code));
            }
            Some(pb::pairing_update::Event::Paired(info)) => {
                paired_jid = info.jid.map(|j| j.value).unwrap_or_default();
                println!("\n✅ PAIRED as {paired_jid}");
                break;
            }
            Some(pb::pairing_update::Event::Error(e)) => {
                println!("❌ PAIR ERROR: {}", e.message);
                return Ok(());
            }
            _ => {}
        }
    }

    // wait CONNECTED
    let mut own = paired_jid.clone();
    for _ in 0..100 {
        if let Ok(r) = account.get_account_status(acct.clone()).await {
            let s = r.into_inner();
            if s.state == pb::ConnectionState::Connected as i32 {
                if let Some(j) = s.jid {
                    own = j.value;
                }
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    println!("connected; own jid = {own}");

    // ---- SetProfilePicture (square JPEG) + restore ----
    let jid_req = pb::JidRequest {
        account: Some(acct.clone()),
        jid: own.clone(),
    };
    let orig_url = contacts
        .get_profile_picture(jid_req)
        .await
        .map(|r| r.into_inner().url)
        .unwrap_or_default();
    let orig_bytes = if orig_url.is_empty() {
        None
    } else {
        fetch_url_bytes(&orig_url)
    };
    println!(
        "current profile pic: {}",
        if orig_url.is_empty() {
            "none".into()
        } else {
            format!("url present, fetched={}", orig_bytes.is_some())
        }
    );
    match contacts
        .set_profile_picture(pb::SetProfilePictureRequest {
            account: Some(acct.clone()),
            image: square_jpeg()?,
        })
        .await
    {
        Ok(_) => println!("✅ SetProfilePicture (square JPEG) OK"),
        Err(e) => println!("❌ SetProfilePicture: {}", e.message()),
    }
    // restore
    if let Some(bytes) = orig_bytes {
        match contacts
            .set_profile_picture(pb::SetProfilePictureRequest {
                account: Some(acct.clone()),
                image: bytes,
            })
            .await
        {
            Ok(_) => println!("✅ profile pic restored to original"),
            Err(e) => println!("❌ restore original: {}", e.message()),
        }
    } else {
        match contacts.remove_profile_picture(acct.clone()).await {
            Ok(_) => println!("✅ profile pic restored (removed; had none)"),
            Err(e) => println!("❌ restore(remove): {}", e.message()),
        }
    }

    // ---- DownloadMedia: wait for inbound media, then download ----
    let sub = pb::SubscribeRequest {
        selector: Some(pb::subscribe_request::Selector::Account(acct.clone())),
        replay_from_ring: 0,
    };
    let mut ev = events.subscribe_events(sub).await?.into_inner();
    println!(
        "\n>>> Now send a PHOTO/VIDEO/DOC to the paired number from 62998877474. Waiting up to 180s ...\n"
    );
    let mut descriptor: Option<pb::MediaDescriptor> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
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
            Err(_) => {} // 5s tick, keep waiting
        }
    }

    match descriptor {
        Some(d) => {
            let mime = d.mime_type.clone();
            match media
                .download_media(pb::DownloadMediaRequest {
                    account: Some(acct.clone()),
                    descriptor: Some(d),
                })
                .await
            {
                Ok(s) => {
                    let mut s = s.into_inner();
                    let mut bytes = Vec::new();
                    while let Ok(Some(chunk)) = s.message().await {
                        if let Some(pb::media_chunk::Part::Chunk(b)) = chunk.part {
                            bytes.extend_from_slice(&b);
                        }
                    }
                    let path = "/tmp/wamux-download.bin";
                    let _ = std::fs::write(path, &bytes);
                    println!(
                        "✅ DownloadMedia OK: {} bytes (mime={mime}) saved to {path}",
                        bytes.len()
                    );
                }
                Err(e) => println!("❌ DownloadMedia: {}", e.message()),
            }
        }
        None => println!("❌ DownloadMedia: no inbound media received in the window"),
    }

    Ok(())
}

/// 640x640 JPEG (WhatsApp profile pictures must be square JPEG).
fn square_jpeg() -> anyhow::Result<Vec<u8>> {
    let mut img = image::RgbImage::new(640, 640);
    for (x, y, p) in img.enumerate_pixels_mut() {
        *p = image::Rgb([(x % 256) as u8, (y % 256) as u8, 120]);
    }
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Jpeg)?;
    Ok(buf)
}

fn fetch_url_bytes(url: &str) -> Option<Vec<u8>> {
    let resp = ureq::get(url).call().ok()?;
    let mut buf = Vec::new();
    resp.into_reader()
        .take(16 * 1024 * 1024)
        .read_to_end(&mut buf)
        .ok()?;
    Some(buf)
}

fn write_qr_png(data: &str, path: &str) -> anyhow::Result<()> {
    let code = qrcode::QrCode::new(data.as_bytes())?;
    let width = code.width();
    let colors = code.to_colors();
    let (scale, quiet) = (8usize, 4usize);
    let dim = ((width + quiet * 2) * scale) as u32;
    let mut img = image::GrayImage::from_pixel(dim, dim, image::Luma([255u8]));
    for y in 0..width {
        for x in 0..width {
            if colors[y * width + x] == qrcode::Color::Dark {
                for dy in 0..scale {
                    for dx in 0..scale {
                        img.put_pixel(
                            ((x + quiet) * scale + dx) as u32,
                            ((y + quiet) * scale + dy) as u32,
                            image::Luma([0u8]),
                        );
                    }
                }
            }
        }
    }
    img.save(path)?;
    Ok(())
}

fn ascii_qr(data: &str) -> String {
    match qrcode::QrCode::new(data.as_bytes()) {
        Ok(code) => code
            .render::<qrcode::render::unicode::Dense1x2>()
            .quiet_zone(true)
            .build(),
        Err(_) => String::new(),
    }
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
