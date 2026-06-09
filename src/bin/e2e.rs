//! End-to-end exercise of the wamux socket: connect the paired account, send
//! text + image + document to a target, and stream incoming events to validate
//! reception. Everything goes through the real gRPC socket API.
//!
//! Usage: e2e [socket_path] [target_jid_user]
//!   defaults: /tmp/wamux.sock  5511999999999
//! Requires the wamux daemon running on the socket.

use std::io::Cursor;
use std::time::Duration;

use hyper_util::rt::TokioIo;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;

const EXTERNAL_REF: &str = "pair-socket";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/wamux.sock".to_string());
    let arg_target = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "5511999999999".to_string());
    // Accept a full JID (e.g. "...@c.us") or a bare number (append s.whatsapp.net).
    let target = if arg_target.contains('@') {
        arg_target
    } else {
        format!("{arg_target}@s.whatsapp.net")
    };

    let channel = connect_uds(socket_path).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut messaging = MessagingServiceClient::new(channel.clone());
    let mut events = EventServiceClient::new(channel);

    let account_ref = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(EXTERNAL_REF.to_string())),
    };

    // 1) connect (reconnect from persisted state) and wait until CONNECTED.
    println!("connect_account ...");
    account
        .connect_account(pb::ConnectAccountRequest {
            account: Some(account_ref.clone()),
            backfill_history: false,
        })
        .await?;
    let mut connected = false;
    for _ in 0..100 {
        let status = account
            .get_account_status(account_ref.clone())
            .await?
            .into_inner();
        if status.state == pb::ConnectionState::Connected as i32 {
            connected = true;
            println!("connected; jid={:?}", status.jid.map(|j| j.value));
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    if !connected {
        anyhow::bail!("account did not reach CONNECTED");
    }

    // 2) subscribe to events in the background to validate RECEPTION.
    let sub = pb::SubscribeRequest {
        selector: Some(pb::subscribe_request::Selector::Account(
            account_ref.clone(),
        )),
        replay_from_ring: 0,
    };
    let mut stream = events.subscribe_events(sub).await?.into_inner();
    tokio::spawn(async move {
        println!("[sub] listening for incoming events ...");
        loop {
            match stream.message().await {
                Ok(Some(env)) => print_event(env),
                Ok(None) => {
                    println!("[sub] stream ended");
                    break;
                }
                Err(status) => {
                    println!("[sub] stream error: {}", status.message());
                    break;
                }
            }
        }
    });

    // 3) send text.
    let text = messaging
        .send_text(pb::SendTextRequest {
            account: Some(account_ref.clone()),
            to: Some(pb::Jid {
                value: target.clone(),
            }),
            text: "wamux e2e: mensagem de texto ✅".to_string(),
            mentions: Vec::new(),
            quote: None,
        })
        .await?
        .into_inner();
    println!("[send] text id={}", key_id(text.key));

    // 4) send an image (generated PNG).
    let png = generate_png()?;
    let img_id = send_media(
        &mut messaging,
        &account_ref,
        &target,
        "image/png",
        "image",
        "e2e.png",
        "wamux e2e: imagem ✅",
        png,
    )
    .await?;
    println!("[send] image id={img_id}");

    // 5) send a document.
    let doc = b"wamux e2e: documento de teste\n".to_vec();
    let doc_id = send_media(
        &mut messaging,
        &account_ref,
        &target,
        "text/plain",
        "document",
        "e2e.txt",
        "wamux e2e: documento ✅",
        doc,
    )
    .await?;
    println!("[send] document id={doc_id}");

    println!("\n>>> Now send a WhatsApp message TO the paired account to test reception.");
    println!(">>> Listening for 180s ...\n");
    tokio::time::sleep(Duration::from_secs(180)).await;
    Ok(())
}

fn print_event(env: pb::EventEnvelope) {
    match env.event {
        Some(pb::event_envelope::Event::Message(m)) => {
            let media = m
                .media
                .as_ref()
                .map(|d| d.media_type.clone())
                .unwrap_or_default();
            println!(
                "[recv] from={} chat={} text={:?} media={:?} reaction={:?}",
                m.sender, m.chat, m.text, media, m.reaction
            );
        }
        Some(pb::event_envelope::Event::Receipt(r)) => {
            println!("[recv] receipt type={} ids={:?}", r.r#type, r.message_ids);
        }
        Some(pb::event_envelope::Event::Connection(c)) => {
            println!("[recv] connection state={} {}", c.state, c.detail);
        }
        Some(pb::event_envelope::Event::Presence(p)) => {
            println!(
                "[recv] presence jid={} online={} state={}",
                p.jid, p.online, p.chat_state
            );
        }
        Some(other) => println!("[recv] other event: {other:?}"),
        None => {}
    }
}

fn key_id(key: Option<pb::MessageKey>) -> String {
    key.map(|k| k.id).unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
async fn send_media(
    messaging: &mut MessagingServiceClient<Channel>,
    account: &pb::AccountRef,
    target: &str,
    mime: &str,
    media_type: &str,
    filename: &str,
    caption: &str,
    data: Vec<u8>,
) -> anyhow::Result<String> {
    let header = pb::SendMediaChunk {
        part: Some(pb::send_media_chunk::Part::Header(pb::SendMediaHeader {
            account: Some(account.clone()),
            to: Some(pb::Jid {
                value: target.to_string(),
            }),
            mime_type: mime.to_string(),
            caption: caption.to_string(),
            mentions: Vec::new(),
            quote: None,
            media_type: media_type.to_string(),
            filename: filename.to_string(),
        })),
    };
    let mut chunks = vec![header];
    for chunk in data.chunks(64 * 1024) {
        chunks.push(pb::SendMediaChunk {
            part: Some(pb::send_media_chunk::Part::Chunk(chunk.to_vec())),
        });
    }
    let result = messaging
        .send_media(tokio_stream::iter(chunks))
        .await?
        .into_inner();
    Ok(key_id(result.key))
}

fn generate_png() -> anyhow::Result<Vec<u8>> {
    let mut img = image::RgbImage::new(600, 300);
    for (x, y, pixel) in img.enumerate_pixels_mut() {
        *pixel = image::Rgb([((x / 3) % 256) as u8, (y % 256) as u8, 160]);
    }
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)?;
    Ok(buf)
}

async fn connect_uds(path: String) -> anyhow::Result<Channel> {
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
