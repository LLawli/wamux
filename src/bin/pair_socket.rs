//! Pairing validator that drives the REAL wamux socket (not the library):
//! connects to the daemon's Unix socket via gRPC, calls AccountService
//! CreateAccount + PairWithQr, renders the QR to /tmp/wamux-qr.png and opens it
//! with xdg-open. On pairing, sends a self-message via MessagingService.
//!
//! Usage: pair_socket [socket_path]   (default: /tmp/wamux.sock)
//! Requires the wamux daemon to be running on that socket.

use hyper_util::rt::TokioIo;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;

const EXTERNAL_REF: &str = "pair-socket";
const QR_PNG: &str = "/tmp/wamux-qr.png";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/wamux.sock".to_string());
    let channel = connect_uds(socket_path.clone()).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut messaging = MessagingServiceClient::new(channel);

    // Create the account (ignore "already exists"): PairWithQr resolves by external_ref.
    match account
        .create_account(pb::CreateAccountRequest {
            external_ref: Some(EXTERNAL_REF.to_string()),
        })
        .await
    {
        Ok(resp) => println!("created account uuid={}", resp.into_inner().uuid),
        Err(status) => println!("create_account: {} (reusing existing)", status.message()),
    }

    let account_ref = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(EXTERNAL_REF.to_string())),
    };

    println!("calling PairWithQr over the socket ...");
    let mut stream = account
        .pair_with_qr(pb::PairWithQrRequest {
            account: Some(account_ref.clone()),
            backfill_history: false,
        })
        .await?
        .into_inner();
    let mut opened = false;
    while let Some(update) = stream.message().await? {
        match update.event {
            Some(pb::pairing_update::Event::QrCode(code)) => {
                if let Err(e) = write_qr_png(&code, QR_PNG) {
                    eprintln!("[qr] render failed: {e}");
                } else {
                    println!("[qr] new QR -> {QR_PNG}");
                    if !opened {
                        let _ = std::process::Command::new("xdg-open").arg(QR_PNG).spawn();
                        opened = true;
                    }
                }
                println!("{}", ascii_qr(&code));
            }
            Some(pb::pairing_update::Event::PairCode(code)) => println!("pair code: {code}"),
            Some(pb::pairing_update::Event::Paired(info)) => {
                let jid = info.jid.map(|j| j.value).unwrap_or_default();
                println!("\n✅ PAIRED as {jid} (push_name={})", info.push_name);
                send_self(&mut messaging, account_ref.clone(), jid).await;
                break;
            }
            Some(pb::pairing_update::Event::Error(err)) => {
                println!("\n❌ PAIR ERROR: {}", err.message);
                break;
            }
            None => {}
        }
    }
    Ok(())
}

async fn send_self(
    messaging: &mut MessagingServiceClient<Channel>,
    account: pb::AccountRef,
    jid: String,
) {
    if jid.is_empty() {
        return;
    }
    let request = pb::SendTextRequest {
        account: Some(account),
        to: Some(pb::Jid { value: jid }),
        text: "wamux: pareamento via socket + envio OK ✅".to_string(),
        mentions: Vec::new(),
        quote: None,
        link_preview: None,
        ephemeral_seconds: 0,
    };
    match messaging.send_text(request).await {
        Ok(resp) => {
            let id = resp.into_inner().key.map(|k| k.id).unwrap_or_default();
            println!("[send] self-message sent id={id}");
        }
        Err(status) => println!("[send] failed: {}", status.message()),
    }
}

async fn connect_uds(path: String) -> anyhow::Result<Channel> {
    // The authority is ignored for UDS; the connector dials the socket.
    let channel = Endpoint::try_from("http://[::1]:50051")?
        .connect_with_connector(service_fn(move |_: Uri| {
            let path = path.clone();
            async move {
                let stream = tokio::net::UnixStream::connect(path).await?;
                Ok::<_, std::io::Error>(TokioIo::new(stream))
            }
        }))
        .await?;
    Ok(channel)
}

fn write_qr_png(data: &str, path: &str) -> anyhow::Result<()> {
    let code = qrcode::QrCode::new(data.as_bytes())?;
    let width = code.width();
    let colors = code.to_colors();
    let scale = 8usize;
    let quiet = 4usize;
    let dim = ((width + quiet * 2) * scale) as u32;
    let mut img = image::GrayImage::from_pixel(dim, dim, image::Luma([255u8]));
    for y in 0..width {
        for x in 0..width {
            if colors[y * width + x] == qrcode::Color::Dark {
                for dy in 0..scale {
                    for dx in 0..scale {
                        let px = ((x + quiet) * scale + dx) as u32;
                        let py = ((y + quiet) * scale + dy) as u32;
                        img.put_pixel(px, py, image::Luma([0u8]));
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
