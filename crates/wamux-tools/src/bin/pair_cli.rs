//! Interactive pairing validator.
//!
//! Modes:
//!   pair_cli qr                 -> QR pairing: renders the QR to /tmp/wamux-qr.png
//!                                  (+ ASCII) on each refresh; scan with the phone.
//!   pair_cli <intl_digits>      -> PAIR CODE: requests the 8-digit code ONCE
//!                                  (rate-limited; never retried).
//!
//! Once paired, sends a self-message to validate the send path too, then exits.
//! Env: WAMUX_REF (required, the account to pair or reuse), DATABASE_URL
//! (defaults to the local docker postgres). Runs the account in-process, so it
//! needs no daemon.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast::{Receiver, error::RecvError};
use whatsapp_rust::pair_code::PairCodeOptions;

use wamux::domain::messaging;
use wamux::proto::v1 as pb;
use wamux::state::AccountHandle;
use wamux_tools::inproc::{database_url_from, init_tracing, open_registry, resolve_or_create};
use wamux_tools::live_env::{account_ref_from, process_env};
use wamux_tools::qr::{ascii_qr, open_in_viewer, write_qr_png};
use wamux_tools::report::Report;
use wamux_types::{Jid, OutgoingText};

const QR_PNG: &str = "/tmp/wamux-qr.png";
/// How long a pairing may take before the run gives up and fails.
const PAIRING_DEADLINE: Duration = Duration::from_secs(600);

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let external_ref: String = account_ref_from(&process_env)?;
    let database_url: String = database_url_from(&process_env);
    let arg = std::env::args().nth(1).unwrap_or_default();
    let qr_mode = arg.is_empty() || arg.eq_ignore_ascii_case("qr");
    let phone = if qr_mode { String::new() } else { arg };
    init_tracing("warn,wamux=info,whatsapp_rust=info");

    let registry = open_registry(&database_url).await?;
    let (handle, created) = resolve_or_create(&registry, &external_ref).await?;
    println!(
        "{} account {} (device_id={})",
        if created { "Created" } else { "Reusing" },
        handle.uuid,
        handle.device_id
    );
    let events = handle.subscribe();
    println!(
        "Mode: {} | connecting ...",
        if qr_mode { "QR" } else { "PAIR CODE" }
    );
    registry.connect(&handle, None, true).await?;
    if !qr_mode {
        spawn_pair_request(handle.clone(), phone.clone());
    }
    let mut report = Report::new();
    let paired =
        tokio::time::timeout(PAIRING_DEADLINE, watch_events(events, qr_mode, &mut report)).await;
    match paired {
        Ok(Some(jid)) => send_self_message(&handle, &phone, jid, &mut report).await,
        Ok(None) => {}
        Err(_) => report.fail("Pairing", format!("not paired within {PAIRING_DEADLINE:?}")),
    }
    Ok(report.finish())
}

/// Print events until the phone pairs (returns its jid) or pairing fails.
async fn watch_events(
    mut events: Receiver<pb::EventEnvelope>,
    qr_mode: bool,
    report: &mut Report,
) -> Option<String> {
    loop {
        let envelope = match events.recv().await {
            Ok(env) => env,
            Err(RecvError::Lagged(n)) => {
                eprintln!("(lagged {n} events)");
                continue;
            }
            Err(RecvError::Closed) => {
                report.fail("Pairing", "event stream closed before pairing");
                return None;
            }
        };
        match envelope.event {
            Some(pb::event_envelope::Event::Pairing(update)) => {
                if let Some(outcome) = show_pairing(update, qr_mode, report) {
                    return outcome;
                }
            }
            Some(pb::event_envelope::Event::Connection(state)) => {
                let name = pb::ConnectionState::try_from(state.state)
                    .map(|s| format!("{s:?}"))
                    .unwrap_or_else(|_| state.state.to_string());
                println!("[conn] {name} {}", state.detail);
            }
            Some(pb::event_envelope::Event::Message(message)) => {
                println!("[msg] from {}: {}", message.sender, message.text);
            }
            _ => {}
        }
    }
}

/// `Some(outcome)` ends the watch: the paired jid, or `None` after a recorded error.
fn show_pairing(
    update: pb::PairingUpdate,
    qr_mode: bool,
    report: &mut Report,
) -> Option<Option<String>> {
    match update.event {
        Some(pb::pairing_update::Event::QrCode(code)) if qr_mode => on_qr(&code),
        Some(pb::pairing_update::Event::PairCode(code)) => {
            println!("\n================= PAIR CODE =================");
            println!("                {code}");
            println!("============================================\n");
        }
        Some(pb::pairing_update::Event::Paired(info)) => {
            let jid: String = info.jid.map(|j| j.value).unwrap_or_default();
            println!("\nPAIRED as {jid} (business_name={})", info.business_name);
            report.verify("Pairing", !jid.is_empty(), format!("paired as {jid}"));
            return Some(Some(jid).filter(|jid| !jid.is_empty()));
        }
        Some(pb::pairing_update::Event::Error(err)) => {
            report.fail("Pairing", err.message);
            return Some(None);
        }
        _ => {}
    }
    None
}

fn on_qr(code: &str) {
    let path = std::path::Path::new(QR_PNG);
    match write_qr_png(code, path) {
        Ok(()) => {
            println!("[qr] new QR written to {QR_PNG} (scan with the phone)");
            open_in_viewer(path);
        }
        Err(e) => eprintln!("[qr] failed to render PNG: {e}"),
    }
    match ascii_qr(code) {
        Ok(text) => println!("{text}"),
        Err(e) => eprintln!("[qr] {e}"),
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

/// After pairing, wait for the link to settle then send a self-message. A
/// self-chat has no receipt and no fan-out (CLAUDE.md), so it is `accepted`.
async fn send_self_message(
    handle: &Arc<AccountHandle>,
    phone: &str,
    paired_jid: String,
    report: &mut Report,
) {
    tokio::time::sleep(Duration::from_secs(6)).await;
    let target = if phone.is_empty() {
        paired_jid
    } else {
        format!("{phone}@s.whatsapp.net")
    };
    let Some(client) = handle.client().await else {
        return report.fail("Messaging.SendText(self)", "no client available");
    };
    let jid = match Jid::parse(&target) {
        Ok(jid) => jid,
        Err(e) => return report.fail("Messaging.SendText(self)", format!("bad jid: {e}")),
    };
    let text = OutgoingText {
        text: "wamux: pareamento + envio OK".to_string(),
        ..Default::default()
    };
    match messaging::send_text(&client, jid, &text).await {
        Ok(result) => report.accepted(
            "Messaging.SendText(self)",
            format!("id={}", result.message_id),
        ),
        Err(e) => report.fail("Messaging.SendText(self)", e.to_string()),
    }
}
