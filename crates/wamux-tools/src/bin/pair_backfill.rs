//! Validate the InitialBootstrap history dump end-to-end: pair a FRESH account
//! via QR with backfill ENABLED (skip_history=false), then watch the history
//! sync the phone pushes right after linking. Registry-direct (not over the
//! socket) because the socket pairing RPCs don't expose the backfill flag yet.
//!
//! Opens the QR PNG with xdg-open on the first refresh; scan it with the phone.
//!
//! Env: WAMUX_REF (required, the account to pair or reuse), DATABASE_URL
//! (defaults to the local docker postgres).
//! Usage: pair_backfill [watch_secs]   (default 120)

use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use whatsapp_rust::buffa::{Enumeration as _, Message as _};
use whatsapp_rust::waproto::whatsapp as wa;

use wamux::proto::v1 as pb;
use wamux_tools::inproc::{database_url_from, init_tracing, open_registry, resolve_or_create};
use wamux_tools::live_env::{account_ref_from, process_env};
use wamux_tools::qr::{ascii_qr, open_in_viewer, write_qr_png};
use wamux_tools::report::Report;

const QR_PNG: &str = "/tmp/wamux-qr.png";

/// What the history watch counted.
#[derive(Default)]
struct HistoryTally {
    paired: bool,
    pair_error: Option<String>,
    events: usize,
    conversations: usize,
    messages: usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let external_ref: String = account_ref_from(&process_env)?;
    let database_url: String = database_url_from(&process_env);
    let watch_secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);
    init_tracing("warn,wamux=info,whatsapp_rust=info");

    let registry = open_registry(&database_url).await?;
    let (handle, created) = resolve_or_create(&registry, &external_ref).await?;
    println!(
        "{} account {} (device_id={})",
        if created { "created" } else { "reusing" },
        handle.uuid,
        handle.device_id
    );
    let events = handle.subscribe();
    // QR mode (pair_code=None), backfill ON (skip_history=false).
    println!("connecting with backfill ON; scan the QR when it opens ...");
    registry.connect(&handle, None, false).await?;

    let tally = watch_history(events, watch_secs).await;
    println!(
        "\n=== summary ===\npaired: {}\nhistory_sync events: {}\nconversations: {}\nmessages: {}",
        tally.paired, tally.events, tally.conversations, tally.messages
    );
    println!(
        "(account '{external_ref}' left paired; run `e2e_all`/`logout_e2e` against it or DeleteAccount to clean up.)"
    );
    Ok(report_of(&tally).finish())
}

/// A pairing error or no pairing at all fails; so does a pairing that never
/// got a history chunk, since that is the whole point of this probe.
fn report_of(tally: &HistoryTally) -> Report {
    let mut report = Report::new();
    if let Some(message) = &tally.pair_error {
        report.fail("Pairing", message.clone());
        return report;
    }
    report.verify(
        "Pairing",
        tally.paired,
        "the phone completed the QR pairing",
    );
    if tally.paired {
        report.verify(
            "HistorySyncEvent InitialBootstrap",
            tally.events > 0,
            format!(
                "{} events, {} conversations, {} messages",
                tally.events, tally.conversations, tally.messages
            ),
        );
    }
    report
}

/// Show the QR until paired, then count history events for `watch_secs`.
async fn watch_history(
    mut events: tokio::sync::broadcast::Receiver<pb::EventEnvelope>,
    watch_secs: u64,
) -> HistoryTally {
    let mut tally = HistoryTally::default();
    let mut qr_opened = false;
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
        match env.event {
            Some(pb::event_envelope::Event::Pairing(u)) => match u.event {
                Some(pb::pairing_update::Event::QrCode(code)) => {
                    render_qr(&code, &mut qr_opened);
                }
                Some(pb::pairing_update::Event::Paired(info)) => {
                    let jid = info.jid.map(|j| j.value).unwrap_or_default();
                    println!("\nPAIRED as {jid} (business_name={})", info.business_name);
                    println!("watching {watch_secs}s for the InitialBootstrap history dump ...\n");
                    tally.paired = true;
                    deadline = tokio::time::Instant::now() + Duration::from_secs(watch_secs);
                }
                Some(pb::pairing_update::Event::Error(e)) => {
                    println!("\nPAIR ERROR: {}", e.message);
                    tally.pair_error = Some(e.message);
                    break;
                }
                _ => {}
            },
            Some(pb::event_envelope::Event::HistorySync(h)) => count_history(&mut tally, &h),
            Some(pb::event_envelope::Event::Connection(c)) => {
                let name = pb::ConnectionState::try_from(c.state)
                    .map(|s| format!("{s:?}"))
                    .unwrap_or_else(|_| c.state.to_string());
                println!("[conn] {name} {}", c.detail);
            }
            _ => {}
        }
    }
    tally
}

fn count_history(tally: &mut HistoryTally, h: &pb::HistorySyncEvent) {
    tally.events += 1;
    let (convs, msgs) = decode_counts(&h.raw);
    tally.conversations += convs;
    tally.messages += msgs;
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

fn render_qr(code: &str, opened: &mut bool) {
    let path = Path::new(QR_PNG);
    if let Err(e) = write_qr_png(code, path) {
        eprintln!("[qr] PNG render failed: {e}");
    } else if !*opened {
        open_in_viewer(path);
        *opened = true;
    }
    match ascii_qr(code) {
        Ok(text) => println!("{text}"),
        Err(e) => eprintln!("[qr] {e}"),
    }
}
