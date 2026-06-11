//! Interactive pairing validator.
//!
//! Modes:
//!   pair_cli qr                 -> QR pairing: renders the QR to /tmp/wamux-qr.png
//!                                  (+ ASCII) on each refresh; scan with the phone.
//!   pair_cli <intl_digits>      -> PAIR CODE: requests the 8-digit code ONCE
//!                                  (rate-limited; never retried).
//!
//! Once paired, sends a self-message to validate the send path too.
//! Env: DATABASE_URL (defaults to the local docker postgres).

use std::sync::Arc;
use std::time::Duration;

use tracing_subscriber::EnvFilter;
use whatsapp_rust::pair_code::PairCodeOptions;

use wamux::domain::{jid_parse, messaging};
use wamux::proto::v1 as pb;
use wamux::state::{AccountHandle, AccountRegistry, RegistryTuning};
use wamux::storage;

const EXTERNAL_REF: &str = "pair-cli";
const QR_PNG: &str = "/tmp/wamux-qr.png";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("warn,wamux=info,whatsapp_rust=info")),
        )
        .init();

    let arg = std::env::args().nth(1).unwrap_or_default();
    let qr_mode = arg.is_empty() || arg.eq_ignore_ascii_case("qr");
    let phone = if qr_mode { String::new() } else { arg };

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wamux:wamux@localhost:5433/wamux".to_string());
    let pool = storage::postgres::connect(&database_url, 8).await?;
    storage::postgres::run_migrations(&pool).await?;
    let registry = Arc::new(AccountRegistry::new(pool, RegistryTuning::with_ring(256)));
    registry.load_existing().await?;

    let external_ref = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(EXTERNAL_REF.to_string())),
    };
    let handle = match registry.resolve(Some(&external_ref)) {
        Ok(handle) => {
            println!(
                "Reusing account {} (device_id={})",
                handle.uuid, handle.device_id
            );
            handle
        }
        Err(_) => {
            let handle = registry.create_account(Some(EXTERNAL_REF)).await?;
            println!(
                "Created account {} (device_id={})",
                handle.uuid, handle.device_id
            );
            handle
        }
    };

    let mut events = handle.subscribe();
    println!(
        "Mode: {} | connecting ...",
        if qr_mode { "QR" } else { "PAIR CODE" }
    );
    registry.connect(&handle, None, true).await?;
    if !qr_mode {
        spawn_pair_request(handle.clone(), phone.clone());
    }

    loop {
        let envelope = match events.recv().await {
            Ok(env) => env,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                eprintln!("(lagged {n} events)");
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        };
        let Some(event) = envelope.event else {
            continue;
        };
        match event {
            pb::event_envelope::Event::Pairing(update) => match update.event {
                Some(pb::pairing_update::Event::QrCode(code)) if qr_mode => {
                    on_qr(&code);
                }
                Some(pb::pairing_update::Event::QrCode(_)) => {}
                Some(pb::pairing_update::Event::PairCode(code)) => {
                    println!("\n================= PAIR CODE =================");
                    println!("                {code}");
                    println!("============================================\n");
                }
                Some(pb::pairing_update::Event::Paired(info)) => {
                    let jid = info.jid.map(|j| j.value).unwrap_or_default();
                    println!(
                        "\n✅ PAIRED as {jid} (business_name={})",
                        info.business_name
                    );
                    let target = if phone.is_empty() {
                        jid
                    } else {
                        format!("{phone}@s.whatsapp.net")
                    };
                    spawn_self_message(handle.clone(), target);
                }
                Some(pb::pairing_update::Event::Error(err)) => {
                    println!("\n❌ PAIR ERROR: {}", err.message);
                }
                None => {}
            },
            pb::event_envelope::Event::Connection(state) => {
                let name = pb::ConnectionState::try_from(state.state)
                    .map(|s| format!("{s:?}"))
                    .unwrap_or_else(|_| state.state.to_string());
                println!("[conn] {name} {}", state.detail);
            }
            pb::event_envelope::Event::Message(message) => {
                println!("[msg] from {}: {}", message.sender, message.text);
            }
            _ => {}
        }
    }
    Ok(())
}

fn on_qr(code: &str) {
    match write_qr_png(code, QR_PNG) {
        Ok(()) => println!("[qr] new QR written to {QR_PNG} (scan with the phone)"),
        Err(e) => eprintln!("[qr] failed to render PNG: {e}"),
    }
    println!("{}", ascii_qr(code));
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

/// Request the pair code EXACTLY ONCE (rate-limited; never retried).
fn spawn_pair_request(handle: Arc<AccountHandle>, phone: String) {
    tokio::spawn(async move {
        let client = loop {
            if let Some(client) = handle.client().await {
                break client;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        };
        tokio::time::sleep(Duration::from_secs(3)).await;
        if handle.current_state() == pb::ConnectionState::Connected {
            println!("[pair] already logged in; no code needed");
            return;
        }
        println!("[pair] requesting code (single attempt) ...");
        match client.pair_with_code(make_options(&phone)).await {
            Ok(code) => println!("[pair] code requested OK: {code}"),
            Err(e) => {
                eprintln!("[pair] request failed: {e}");
                let mut source = std::error::Error::source(&e);
                while let Some(s) = source {
                    eprintln!("        caused by: {s}");
                    source = s.source();
                }
                eprintln!("[pair] NOT retrying (code requests are rate-limited).");
            }
        }
    });
}

fn make_options(phone: &str) -> PairCodeOptions {
    PairCodeOptions {
        phone_number: phone.to_string(),
        show_push_notification: true,
        ..Default::default()
    }
}

/// After pairing, wait for the link to settle then send a self-message.
fn spawn_self_message(handle: Arc<AccountHandle>, target: String) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(6)).await;
        let Some(client) = handle.client().await else {
            println!("[send] no client available for self-message");
            return;
        };
        match jid_parse::parse_jid(&target) {
            Ok(jid) => match messaging::send_text(
                &client,
                jid,
                &pb::SendTextRequest {
                    text: "wamux: pareamento + envio OK ✅".to_string(),
                    ..Default::default()
                },
            )
            .await
            {
                Ok(result) => println!("[send] self-message sent, id={}", result.message_id),
                Err(e) => println!("[send] self-message failed: {e}"),
            },
            Err(e) => println!("[send] bad self jid: {e}"),
        }
    });
}
