//! Sprint 3 M4: probe one REAL WhatsApp connection (the secondary account) for
//! round-trip latency while N fake clients are held against the local mock WSS
//! server. The probe sends a text to a single operator-chosen destination and
//! times the delivery receipt; it runs once as a baseline (no load) and again
//! with the N fakes connected, so you can read the latency cost of the load.
//!
//! SAFETY: the destination is taken from `WAMUX_LIVE_DEST` (one JID, e.g.
//! `<digits>@s.whatsapp.net`); the bin only ever sends there, refuses to run if
//! it is unset, and refuses the connected account's own number, so it can
//! never fan out to arbitrary numbers. The legacy server spelling is refused
//! when the env is read (issue #4).
//!
//! Registry-direct, two registries on one shared Postgres pool: `real_registry`
//! talks to the real endpoint (no `ws_url_override`); `mock_registry` points the
//! N fakes at the in-process mock. Needs the `stress` feature for the mock.
//!
//! Usage: stress_live [n_fakes] [probes_per_phase]   (defaults: 199 5)
//! Env: WAMUX_REF (required, the real account), WAMUX_LIVE_DEST (required),
//!      DATABASE_URL (default local docker pg).
//!
//! Run: `WAMUX_REF=<ref> WAMUX_LIVE_DEST=<jid> cargo run --features stress --bin stress_live -- 199 5`
//!
//! A probe that gets no delivery receipt in 30s is a failed check, and the
//! process exits non-zero.

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::{Client, Jid};

use wacore::store::Device;
use wacore::store::traits::DeviceStore;
use wamux::proto::v1 as pb;
use wamux::state::{AccountHandle, AccountRegistry, RegistryTuning};
use wamux::storage::sql::{SqlBackend, SqlPool};
use wamux::stress::MockWaServer;
use wamux_tools::delivery::is_delivery_receipt;
use wamux_tools::inproc::{
    database_url_from, init_tracing, load_registry, open_engine, resolve_or_create,
};
use wamux_tools::live_env::{account_ref_from, live_dest_from, process_env, refuse_own_number};
use wamux_tools::qr::{ascii_qr, open_in_viewer, write_qr_png};
use wamux_tools::report::Report;

const QR_PNG: &str = "/tmp/wamux-qr.png";
/// How long a probe waits for its delivery receipt.
const PROBE_WINDOW: Duration = Duration::from_secs(30);

type ReceiptMap = Arc<Mutex<HashMap<String, Instant>>>;

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    // The whole config first: nothing is opened until every variable is valid.
    let external_ref: String = account_ref_from(&process_env)?;
    let allowed_dest: String = live_dest_from(&process_env)?;
    let database_url: String = database_url_from(&process_env);
    let n_fakes: usize = arg(1).and_then(|s| s.parse().ok()).unwrap_or(199);
    let probes: usize = arg(2).and_then(|s| s.parse().ok()).unwrap_or(5);
    let dest: Jid = allowed_dest
        .parse()
        .map_err(|_| anyhow::anyhow!("WAMUX_LIVE_DEST is not a valid JID: {allowed_dest}"))?;
    init_tracing("warn,wamux=info");
    println!(
        "stress_live: real account '{external_ref}', {n_fakes} fakes, {probes} probes/phase, dest {allowed_dest}"
    );

    // One engine over the shared pool; both registries mint backends from it.
    let engine = open_engine(&database_url).await?;
    let pool = engine.pool().clone();

    // --- real account (real WhatsApp endpoint: no ws_url_override) ---
    let real_registry = load_registry(engine.clone(), RegistryTuning::with_ring(512)).await?;
    let (real, created) = resolve_or_create(&real_registry, &external_ref).await?;
    println!(
        "{} account {} (device_id={})",
        if created {
            "created (QR pairing required)"
        } else {
            "reusing"
        },
        real.uuid,
        real.device_id
    );

    // Drain the real account's events into a receipt-arrival map (id -> Instant).
    let receipts: ReceiptMap = Arc::new(Mutex::new(HashMap::new()));
    spawn_receipt_collector(real.subscribe(), receipts.clone());

    connect_real(&real_registry, &real).await?;
    let client = real
        .client()
        .await
        .ok_or_else(|| anyhow::anyhow!("real account has no live client after connect"))?;
    // No pn means refuse_own_number would compare against "" and pass silently.
    let own: String = client
        .pn()
        .map(|jid| jid.to_string())
        .ok_or_else(|| anyhow::anyhow!("the logged-in client has no phone jid (pn); refusing to probe without the own-number guard"))?;
    refuse_own_number(&own, &allowed_dest)?;

    let mut report = Report::new();
    // --- baseline: probe with NO load ---
    println!("\n=== baseline (no load) ===");
    let baseline = run_probes(&client, &dest, probes, &receipts, "baseline").await;
    record_probes(&mut report, "baseline", &baseline);

    // --- bring up the mock + N fakes ---
    println!("\n=== bringing up {n_fakes} fake connections ===");
    let mock = MockWaServer::start().await?;
    let mock_registry = Arc::new(AccountRegistry::new(
        engine.clone(),
        RegistryTuning {
            ws_url_override: Some(mock.ws_url()),
            graceful_stop_timeout: Duration::from_millis(500),
            ..RegistryTuning::default()
        },
    ));
    let fakes = provision_and_connect_fakes(&mock_registry, &pool, n_fakes).await?;
    wait_for_handshakes(&mock, n_fakes).await;
    report.verify(
        "mock handshakes",
        mock.handshakes_completed() >= n_fakes,
        format!(
            "{} fakes connected (mock handshakes={}, wanted {n_fakes})",
            mock_registry.connected_count(),
            mock.handshakes_completed()
        ),
    );

    // --- under load: same probe with N fakes held ---
    println!("\n=== under load ({n_fakes} fakes) ===");
    let under = run_probes(&client, &dest, probes, &receipts, "under-load").await;
    record_probes(&mut report, "under-load", &under);

    summarize("baseline", &baseline);
    summarize("under-load", &under);

    // --- cleanup the fakes; leave the real account paired ---
    println!("\ncleaning up {} fakes ...", fakes.len());
    for h in &fakes {
        let _ = mock_registry.delete(h).await;
    }
    real_registry.disconnect(&real).await;
    println!("done. real account '{external_ref}' left paired.");
    Ok(report.finish())
}

fn arg(n: usize) -> Option<String> {
    std::env::args().nth(n)
}

/// One check per probe: a probe that timed out is a failure, never skipped.
fn record_probes(report: &mut Report, label: &str, samples: &[Option<Duration>]) {
    for (i, sample) in samples.iter().enumerate() {
        let name = format!("probe {label} #{i}");
        match sample {
            Some(rtt) => report.pass(
                &name,
                format!("receipt RTT {:.0} ms", rtt.as_secs_f64() * 1000.0),
            ),
            None => report.fail(&name, format!("no delivery receipt in {PROBE_WINDOW:?}")),
        }
    }
}

/// Connect the real account and wait until it is LOGGED IN, rendering a QR if
/// the account still needs pairing. Waiting on `is_logged_in()` (not the socket
/// `Connected` event, which fires *before* pairing) is what makes a fresh-QR run
/// correct: the lib emits `Connected` on socket-up, then QR, then login.
async fn connect_real(
    registry: &Arc<AccountRegistry>,
    handle: &Arc<AccountHandle>,
) -> anyhow::Result<()> {
    let mut events = handle.subscribe();
    registry.connect(handle, None, true).await?;
    println!("connecting (scan the QR if one opens) ...");

    let client = handle
        .client()
        .await
        .ok_or_else(|| anyhow::anyhow!("no client after connect"))?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
    loop {
        if client.is_logged_in() {
            println!("real account LOGGED IN");
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            anyhow::bail!("real account did not log in within 600s");
        }
        match tokio::time::timeout(remaining.min(Duration::from_secs(2)), events.recv()).await {
            Ok(Ok(env)) => show_pairing_qr(env),
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(_)) => anyhow::bail!("event stream closed before login"),
            Err(_) => {} // tick: re-check is_logged_in
        }
    }
}

fn show_pairing_qr(env: pb::EventEnvelope) {
    if let Some(pb::event_envelope::Event::Pairing(u)) = env.event
        && let Some(pb::pairing_update::Event::QrCode(code)) = u.event
    {
        render_qr(&code);
    }
}

/// Provision N registered devices and connect them to the mock. Mirrors the M3
/// test: each device has `pn` set + persisted so it logs in via the mock's
/// `<success>`.
async fn provision_and_connect_fakes(
    registry: &Arc<AccountRegistry>,
    pool: &SqlPool,
    n: usize,
) -> anyhow::Result<Vec<Arc<AccountHandle>>> {
    let tag = uuid::Uuid::new_v4();
    let mut fakes = Vec::with_capacity(n);
    for i in 0..n {
        let h = registry
            .create_account(Some(&format!("stress-m4-{tag}-{i}")))
            .await?;
        let mut device = Device::new();
        device.pn = Some(format!("5511{:09}@s.whatsapp.net", 100_000_000 + i).parse()?);
        device.push_name = "Stress".to_string();
        SqlBackend::new(pool.clone(), h.device_id)
            .save(&device)
            .await?;
        fakes.push(h);
    }
    for h in &fakes {
        registry.connect(h, None, true).await?;
    }
    Ok(fakes)
}

async fn wait_for_handshakes(mock: &MockWaServer, n: usize) {
    for _ in 0..600 {
        if mock.handshakes_completed() >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Background task: record the first delivery-receipt arrival time per message
/// id. Only `delivered`/`read`/`played` count; a `sender` or `retry` receipt
/// is not a delivery (CLAUDE.md).
fn spawn_receipt_collector(
    mut events: tokio::sync::broadcast::Receiver<pb::EventEnvelope>,
    receipts: ReceiptMap,
) {
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(env) => record_receipts(&env, &receipts).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
    });
}

async fn record_receipts(env: &pb::EventEnvelope, receipts: &ReceiptMap) {
    let Some(pb::event_envelope::Event::Receipt(r)) = &env.event else {
        return;
    };
    let now = Instant::now();
    let mut map = receipts.lock().await;
    for id in r
        .message_ids
        .iter()
        .filter(|id| is_delivery_receipt(env, id))
    {
        map.entry(id.clone()).or_insert(now);
    }
}

/// Send `count` probe texts to the primary, timing each until its receipt lands.
async fn run_probes(
    client: &Client,
    dest: &Jid,
    count: usize,
    receipts: &ReceiptMap,
    label: &str,
) -> Vec<Option<Duration>> {
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let text = format!("wamux M4 probe ({label} #{i})");
        let rtt = probe_once(client, dest.clone(), &text, receipts).await;
        match rtt {
            Some(d) => println!(
                "  {label} #{i}: receipt RTT {:.0} ms",
                d.as_secs_f64() * 1000.0
            ),
            None => println!("  {label} #{i}: TIMEOUT (no receipt in 30s, primary offline?)"),
        }
        out.push(rtt);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    out
}

/// One probe: send, then poll the receipt map for this message id (30s cap).
async fn probe_once(
    client: &Client,
    dest: Jid,
    text: &str,
    receipts: &ReceiptMap,
) -> Option<Duration> {
    let t0 = Instant::now();
    let message = wa::Message {
        conversation: Some(text.to_string()),
        ..Default::default()
    };
    let id = client.send_message(dest, message).await.ok()?.message_id;

    let deadline = t0 + PROBE_WINDOW;
    loop {
        if let Some(&t1) = receipts.lock().await.get(&id) {
            return Some(t1.saturating_duration_since(t0));
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Print min/median/max over the probes that landed, plus the timeout count.
fn summarize(label: &str, samples: &[Option<Duration>]) {
    let mut ms: Vec<f64> = samples
        .iter()
        .filter_map(|d| d.map(|d| d.as_secs_f64() * 1000.0))
        .collect();
    let timeouts = samples.len() - ms.len();
    if ms.is_empty() {
        println!("[{label}] no receipts ({timeouts} timeouts)");
        return;
    }
    ms.sort_by(f64::total_cmp);
    let median = ms[ms.len() / 2];
    println!(
        "[{label}] n={} min={:.0} median={:.0} max={:.0} ms  ({timeouts} timeouts)",
        ms.len(),
        ms[0],
        median,
        ms[ms.len() - 1],
    );
}

fn render_qr(code: &str) {
    let path = std::path::Path::new(QR_PNG);
    match write_qr_png(code, path) {
        Ok(()) => open_in_viewer(path),
        Err(e) => eprintln!("[qr] PNG render failed: {e}"),
    }
    if let Ok(text) = ascii_qr(code) {
        println!("{text}");
    }
}
