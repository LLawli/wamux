//! Validate the InitialBootstrap history dump end-to-end: pair a FRESH account
//! via QR with backfill ENABLED (skip_history=false), then watch the history
//! sync the phone pushes right after linking. Registry-direct (not over the
//! socket) because the socket pairing RPCs don't expose the backfill flag yet.
//!
//! Opens the QR PNG with xdg-open on the first refresh; scan it with the phone.
//!
//! Usage: pair_backfill [external_ref] [watch_secs]   (default pair-bootstrap 120)
//! Env: DATABASE_URL (defaults to the local docker postgres).

use std::sync::Arc;
use std::time::Duration;

use tracing_subscriber::EnvFilter;
use whatsapp_rust::buffa::{Enumeration as _, Message as _};
use whatsapp_rust::waproto::whatsapp as wa;

use wamux::proto::v1 as pb;
use wamux::state::{AccountRegistry, RegistryTuning};
use wamux::storage;

const QR_PNG: &str = "/tmp/wamux-qr.png";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("warn,wamux=info,whatsapp_rust=info")),
        )
        .init();

    let external_ref = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "pair-bootstrap".to_string());
    let watch_secs: u64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wamux:wamux@localhost:5433/wamux".to_string());
    let engine = Arc::new(storage::postgres::PgStorage::open(&database_url, 8).await?);
    let registry = Arc::new(AccountRegistry::new(engine, RegistryTuning::with_ring(512)));
    registry.load_existing().await?;

    let acct_ref = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(external_ref.clone())),
    };
    let handle = match registry.resolve(Some(&acct_ref)) {
        Ok(h) => {
            println!("reusing account {} (device_id={})", h.uuid, h.device_id);
            h
        }
        Err(_) => {
            let h = registry.create_account(Some(&external_ref)).await?;
            println!("created account {} (device_id={})", h.uuid, h.device_id);
            h
        }
    };

    let mut events = handle.subscribe();
    // QR mode (pair_code=None), backfill ON (skip_history=false).
    println!("connecting with backfill ON; scan the QR when it opens ...");
    registry.connect(&handle, None, false).await?;

    let mut qr_opened = false;
    let mut paired = false;
    let mut n_hist = 0usize;
    let mut n_convs = 0usize;
    let mut n_msgs = 0usize;
    let mut deadline = tokio::time::Instant::now() + Duration::from_secs(600); // until paired
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let env = match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Ok(env)) => env,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                eprintln!("(lagged {n} events)");
                continue;
            }
            _ => break,
        };
        let Some(event) = env.event else { continue };
        match event {
            pb::event_envelope::Event::Pairing(u) => match u.event {
                Some(pb::pairing_update::Event::QrCode(code)) => {
                    render_qr(&code);
                    if !qr_opened {
                        open_qr();
                        qr_opened = true;
                    }
                }
                Some(pb::pairing_update::Event::Paired(info)) => {
                    let jid = info.jid.map(|j| j.value).unwrap_or_default();
                    println!(
                        "\n✅ PAIRED as {jid} (business_name={})",
                        info.business_name
                    );
                    println!("watching {watch_secs}s for the InitialBootstrap history dump ...\n");
                    paired = true;
                    deadline = tokio::time::Instant::now() + Duration::from_secs(watch_secs);
                }
                Some(pb::pairing_update::Event::Error(e)) => {
                    println!("\n❌ PAIR ERROR: {}", e.message);
                    break;
                }
                _ => {}
            },
            pb::event_envelope::Event::HistorySync(h) => {
                n_hist += 1;
                let (convs, msgs) = decode_counts(&h.raw);
                n_convs += convs;
                n_msgs += msgs;
                println!(
                    "[history] type={} chunk={:?} progress={:?} raw={}B convs={} msgs={}",
                    sync_type_name(h.sync_type),
                    h.chunk_order,
                    h.progress,
                    h.raw.len(),
                    convs,
                    msgs
                );
            }
            pb::event_envelope::Event::Connection(c) => {
                let name = pb::ConnectionState::try_from(c.state)
                    .map(|s| format!("{s:?}"))
                    .unwrap_or_else(|_| c.state.to_string());
                println!("[conn] {name} {}", c.detail);
            }
            _ => {}
        }
    }

    println!(
        "\n=== summary ===\npaired: {paired}\nhistory_sync events: {n_hist}\nconversations: {n_convs}\nmessages: {n_msgs}"
    );
    println!(
        "(account '{external_ref}' left paired; run `e2e_all`/`logout_e2e` against it or DeleteAccount to clean up.)"
    );
    Ok(())
}

fn sync_type_name(t: i32) -> String {
    // waproto 0.7 generates closed enums: `from_i32` replaces prost's TryFrom.
    match wa::history_sync::HistorySyncType::from_i32(t) {
        Some(v) => format!("{v:?}"),
        None => t.to_string(),
    }
}

fn decode_counts(raw: &[u8]) -> (usize, usize) {
    match wa::HistorySync::decode_from_slice(raw) {
        Ok(hs) => (
            hs.conversations.len(),
            hs.conversations.iter().map(|c| c.messages.len()).sum(),
        ),
        Err(_) => (0, 0),
    }
}

fn open_qr() {
    match std::process::Command::new("xdg-open").arg(QR_PNG).spawn() {
        Ok(_) => println!("[qr] opened {QR_PNG} with xdg-open"),
        Err(e) => eprintln!("[qr] xdg-open failed ({e}); open {QR_PNG} manually"),
    }
}

fn render_qr(code: &str) {
    if let Err(e) = write_qr_png(code, QR_PNG) {
        eprintln!("[qr] PNG render failed: {e}");
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
