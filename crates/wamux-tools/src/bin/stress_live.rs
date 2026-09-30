//! Sprint 3 M4: probe one REAL WhatsApp connection (the secondary account) for
//! round-trip latency while N fake clients are held against the local mock WSS
//! server. The probe sends a text to a single operator-chosen destination and
//! times the delivery receipt; it runs once as a baseline (no load) and again
//! with the N fakes connected, so you can read the latency cost of the load.
//!
//! SAFETY: the destination is taken from `WAMUX_LIVE_DEST` (one JID, e.g.
//! `5511999999999@c.us`); the bin only ever sends there and refuses to run if
//! it is unset, so it can never fan out to arbitrary numbers.
//!
//! Registry-direct, two registries on one shared Postgres pool: `real_registry`
//! talks to the real endpoint (no `ws_url_override`); `mock_registry` points the
//! N fakes at the in-process mock. Needs the `stress` feature for the mock.
//!
//! Usage: stress_live [external_ref] [n_fakes] [probes_per_phase]
//!   defaults: pair-socket  199  5
//! Env: DATABASE_URL (default local docker pg).
//!
//! Run: `cargo run --features stress --bin stress_live -- pair-socket 199 5`

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::{Client, Jid};

use wacore::store::Device;
use wacore::store::traits::DeviceStore;
use wamux::proto::v1 as pb;
use wamux::state::{AccountRegistry, RegistryTuning};
use wamux::storage;
use wamux::storage::postgres::PgBackend;
use wamux::stress::MockWaServer;

const QR_PNG: &str = "/tmp/wamux-qr.png";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn,wamux=info")),
        )
        .init();

    let external_ref = arg(1).unwrap_or_else(|| "pair-socket".to_string());
    let n_fakes: usize = arg(2).and_then(|s| s.parse().ok()).unwrap_or(199);
    let probes: usize = arg(3).and_then(|s| s.parse().ok()).unwrap_or(5);

    // Safety guard in code, not just convention: the one permitted destination
    // is supplied by the operator at runtime (no hardcoded number); refuse to
    // run without it, and send nowhere else.
    let allowed_dest = std::env::var("WAMUX_LIVE_DEST").map_err(|_| {
        anyhow::anyhow!("set WAMUX_LIVE_DEST to the destination JID (e.g. 5511999999999@c.us)")
    })?;
    let dest: Jid = allowed_dest
        .parse()
        .map_err(|_| anyhow::anyhow!("WAMUX_LIVE_DEST is not a valid JID: {allowed_dest}"))?;
    println!(
        "stress_live: real account '{external_ref}', {n_fakes} fakes, {probes} probes/phase, dest {allowed_dest}"
    );

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wamux:wamux@localhost:5433/wamux".to_string());
    let pool = storage::postgres::connect(&database_url, 16).await?;
    storage::postgres::run_migrations(&pool).await?;
    // One engine over the shared pool; both registries mint backends from it.
    let engine = Arc::new(storage::postgres::PgStorage::from_pool(pool.clone()));

    // --- real account (real WhatsApp endpoint: no ws_url_override) ---
    let real_registry = Arc::new(AccountRegistry::new(
        engine.clone(),
        RegistryTuning::with_ring(512),
    ));
    real_registry.load_existing().await?;
    let real = resolve_or_create(&real_registry, &external_ref).await?;

    // Drain the real account's events into a receipt-arrival map (id -> Instant).
    let receipts: Arc<Mutex<HashMap<String, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
    spawn_receipt_collector(real.subscribe(), receipts.clone());

    connect_real(&real_registry, &real).await?;
    let client = real
        .client()
        .await
        .ok_or_else(|| anyhow::anyhow!("real account has no live client after connect"))?;

    // --- baseline: probe with NO load ---
    println!("\n=== baseline (no load) ===");
    let baseline = run_probes(&client, &dest, probes, &receipts, "baseline").await;

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
    println!(
        "{} fakes connected (mock handshakes={})",
        mock_registry.connected_count(),
        mock.handshakes_completed()
    );

    // --- under load: same probe with N fakes held ---
    println!("\n=== under load ({n_fakes} fakes) ===");
    let under = run_probes(&client, &dest, probes, &receipts, "under-load").await;

    report("baseline", &baseline);
    report("under-load", &under);

    // --- cleanup the fakes; leave the real account paired ---
    println!("\ncleaning up {} fakes ...", fakes.len());
    for h in &fakes {
        let _ = mock_registry.delete(h).await;
    }
    real_registry.disconnect(&real).await;
    println!("done. real account '{external_ref}' left paired.");
    Ok(())
}

fn arg(n: usize) -> Option<String> {
    std::env::args().nth(n)
}

/// Reuse a persisted account by external_ref, or create a fresh one to pair.
async fn resolve_or_create(
    registry: &Arc<AccountRegistry>,
    external_ref: &str,
) -> anyhow::Result<Arc<wamux::state::AccountHandle>> {
    let acct_ref = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(external_ref.to_string())),
    };
    match registry.resolve(Some(&acct_ref)) {
        Ok(h) => {
            println!("reusing account {} (device_id={})", h.uuid, h.device_id);
            Ok(h)
        }
        Err(_) => {
            let h = registry.create_account(Some(external_ref)).await?;
            println!(
                "created account {} (device_id={}); QR pairing required",
                h.uuid, h.device_id
            );
            Ok(h)
        }
    }
}

/// Connect the real account and wait until it is LOGGED IN, rendering a QR if
/// the account still needs pairing. Waiting on `is_logged_in()` (not the socket
/// `Connected` event, which fires *before* pairing) is what makes a fresh-QR run
/// correct: the lib emits `Connected` on socket-up, then QR, then login.
async fn connect_real(
    registry: &Arc<AccountRegistry>,
    handle: &Arc<wamux::state::AccountHandle>,
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
            Ok(Ok(env)) => handle_pairing(env),
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(_)) => anyhow::bail!("event stream closed before login"),
            Err(_) => {} // tick: re-check is_logged_in
        }
    }
}

fn handle_pairing(env: pb::EventEnvelope) {
    if let Some(pb::event_envelope::Event::Pairing(u)) = env.event
        && let Some(pb::pairing_update::Event::QrCode(code)) = u.event
    {
        render_qr(&code);
        open_qr();
    }
}

/// Provision N registered devices and connect them to the mock. Mirrors the M3
/// test: each device has `pn` set + persisted so it logs in via the mock's
/// `<success>`.
async fn provision_and_connect_fakes(
    registry: &Arc<AccountRegistry>,
    pool: &sqlx::PgPool,
    n: usize,
) -> anyhow::Result<Vec<Arc<wamux::state::AccountHandle>>> {
    let tag = uuid::Uuid::new_v4();
    let mut fakes = Vec::with_capacity(n);
    for i in 0..n {
        let h = registry
            .create_account(Some(&format!("stress-m4-{tag}-{i}")))
            .await?;
        let mut device = Device::new();
        device.pn = Some(format!("5511{:09}@s.whatsapp.net", 100_000_000 + i).parse()?);
        device.push_name = "Stress".to_string();
        PgBackend::new(pool.clone(), h.device_id)
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

/// Background task: record the first delivery-receipt arrival time per message id.
fn spawn_receipt_collector(
    mut events: tokio::sync::broadcast::Receiver<pb::EventEnvelope>,
    receipts: Arc<Mutex<HashMap<String, Instant>>>,
) {
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(env) => {
                    if let Some(pb::event_envelope::Event::Receipt(r)) = env.event {
                        let now = Instant::now();
                        let mut map = receipts.lock().await;
                        for id in r.message_ids {
                            map.entry(id).or_insert(now);
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
    });
}

/// Send `count` probe texts to the primary, timing each until its receipt lands.
async fn run_probes(
    client: &Client,
    dest: &Jid,
    count: usize,
    receipts: &Arc<Mutex<HashMap<String, Instant>>>,
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
            None => println!("  {label} #{i}: TIMEOUT (no receipt in 30s — primary offline?)"),
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
    receipts: &Arc<Mutex<HashMap<String, Instant>>>,
) -> Option<Duration> {
    let t0 = Instant::now();
    let message = wa::Message {
        conversation: Some(text.to_string()),
        ..Default::default()
    };
    let id = client.send_message(dest, message).await.ok()?.message_id;

    let deadline = t0 + Duration::from_secs(30);
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
fn report(label: &str, samples: &[Option<Duration>]) {
    let mut ms: Vec<f64> = samples
        .iter()
        .filter_map(|d| d.map(|d| d.as_secs_f64() * 1000.0))
        .collect();
    let timeouts = samples.len() - ms.len();
    if ms.is_empty() {
        println!("[{label}] no receipts ({timeouts} timeouts)");
        return;
    }
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = ms[ms.len() / 2];
    println!(
        "[{label}] n={} min={:.0} median={:.0} max={:.0} ms  ({timeouts} timeouts)",
        ms.len(),
        ms[0],
        median,
        ms[ms.len() - 1],
    );
}

fn open_qr() {
    match std::process::Command::new("xdg-open").arg(QR_PNG).spawn() {
        Ok(_) => println!("[qr] opened {QR_PNG}"),
        Err(e) => eprintln!("[qr] xdg-open failed ({e}); open {QR_PNG} manually"),
    }
}

fn render_qr(code: &str) {
    if let Err(e) = write_qr_png(code, QR_PNG) {
        eprintln!("[qr] PNG render failed: {e}");
    }
    if let Ok(qr) = qrcode::QrCode::new(code.as_bytes()) {
        println!(
            "{}",
            qr.render::<qrcode::render::unicode::Dense1x2>()
                .quiet_zone(true)
                .build()
        );
    }
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
            if colors[y * width + x] != qrcode::Color::Dark {
                continue;
            }
            for dy in 0..scale {
                for dx in 0..scale {
                    let px = ((x + quiet) * scale + dx) as u32;
                    let py = ((y + quiet) * scale + dy) as u32;
                    img.put_pixel(px, py, image::Luma([0u8]));
                }
            }
        }
    }
    img.save(path)?;
    Ok(())
}
