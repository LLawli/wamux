//! Bench client: the minimal "edge" the core needs in order to be doing real
//! work. Connects every account and holds one all-accounts event subscription
//! open, decoding whatever the daemon relays.
//!
//! It exists for the wamux-vs-wacli memory comparison. wacli connects, decodes
//! and processes inside one process; wamux splits that across the daemon and
//! whoever consumes the socket. Measuring the daemon alone, idle with nobody
//! subscribed, would flatter it: nothing would ever be encoded or sent. So this
//! subscribes, and its own footprint is reported too, to be added to the
//! daemon's for an honest total.
//!
//! It deliberately persists NOTHING (the core is a pure relay, and wacli's
//! message history is the difference the comparison has to state, not hide).
//!
//! Usage: bench_client [socket_path]   (default: ~/.local/state/wamux/wamux.sock)

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use hyper_util::rt::TokioIo;
use prost::Message;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;

/// How often the running totals are printed.
const REPORT_EVERY: Duration = Duration::from_secs(60);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket_path = std::env::args().nth(1).unwrap_or_else(default_socket_path);
    let channel = connect_uds(socket_path.clone()).await?;
    let connected = connect_every_account(&channel).await?;
    println!("[bench] {connected} account(s) connected via {socket_path}");

    let mut events = EventServiceClient::new(channel)
        .subscribe_events(pb::SubscribeRequest {
            // Dynamic: accounts paired later join this same stream.
            selector: Some(pb::subscribe_request::Selector::AllAccounts(pb::Empty {})),
            replay_from_ring: 0,
        })
        .await?
        .into_inner();

    let started = Instant::now();
    let mut tally = Tally::default();
    let mut ticker = tokio::time::interval(REPORT_EVERY);
    ticker.tick().await; // the first tick resolves immediately

    println!("[bench] subscribed; consuming events (Ctrl+C to stop)");
    loop {
        tokio::select! {
            message = events.message() => match message? {
                Some(envelope) => tally.record(&envelope),
                None => {
                    println!("[bench] stream closed by the daemon");
                    break;
                }
            },
            _ = ticker.tick() => tally.report(started.elapsed()),
            _ = tokio::signal::ctrl_c() => {
                println!();
                break;
            }
        }
    }
    tally.report(started.elapsed());
    Ok(())
}

/// Running totals. Bytes are the encoded envelope size, i.e. what actually
/// crossed the socket.
#[derive(Default)]
struct Tally {
    events: u64,
    bytes: u64,
    by_kind: BTreeMap<&'static str, u64>,
}

impl Tally {
    fn record(&mut self, envelope: &pb::EventEnvelope) {
        self.events += 1;
        self.bytes += envelope.encoded_len() as u64;
        *self.by_kind.entry(event_kind(envelope)).or_insert(0) += 1;
    }

    fn report(&self, elapsed: Duration) {
        let secs = elapsed.as_secs_f64().max(1.0);
        let kinds: Vec<String> = self
            .by_kind
            .iter()
            .map(|(kind, count)| format!("{kind}={count}"))
            .collect();
        println!(
            "[bench] t={:.0}s events={} ({:.2}/s) bytes={} self_pss={} | {}",
            elapsed.as_secs_f64(),
            self.events,
            self.events as f64 / secs,
            self.bytes,
            self_pss_kib().map_or("?".to_string(), |kib| format!("{kib}KiB")),
            kinds.join(" ")
        );
    }
}

/// One label per envelope variant, for the per-kind breakdown.
fn event_kind(envelope: &pb::EventEnvelope) -> &'static str {
    use pb::event_envelope::Event;
    match &envelope.event {
        Some(Event::Message(_)) => "message",
        Some(Event::Receipt(_)) => "receipt",
        Some(Event::Undecryptable(_)) => "undecryptable",
        Some(Event::Connection(_)) => "connection",
        Some(Event::Pairing(_)) => "pairing",
        Some(Event::Presence(_)) => "presence",
        Some(Event::Group(_)) => "group",
        Some(Event::PushName(_)) => "push_name",
        Some(Event::Contact(_)) => "contact",
        Some(Event::HistorySync(_)) => "history_sync",
        Some(Event::AppState(_)) => "app_state",
        Some(Event::Call(_)) => "call",
        Some(Event::ServerAck(_)) => "server_ack",
        Some(Event::Raw(raw)) => raw_kind(raw),
        None => "empty",
    }
}

/// `subscription_gap` means the fan-out dropped events for this subscriber, so
/// it must not be lumped in with ordinary raw passthrough: it invalidates the
/// event counts for the run.
fn raw_kind(raw: &pb::RawEvent) -> &'static str {
    if raw.kind == "subscription_gap" || raw.kind == "gap" {
        "GAP"
    } else {
        "raw"
    }
}

/// Connect every persisted account, skipping history (the relay-pure default).
/// `ConnectAccount` is idempotent, so a re-run after a daemon restart is safe.
async fn connect_every_account(channel: &Channel) -> anyhow::Result<usize> {
    let mut accounts = AccountServiceClient::new(channel.clone());
    let listed = accounts
        .list_accounts(pb::ListAccountsRequest {})
        .await?
        .into_inner()
        .accounts;
    let mut connected = 0;
    for account in listed {
        let reference = pb::AccountRef {
            r#ref: Some(pb::account_ref::Ref::Uuid(account.uuid.clone())),
        };
        let request = pb::ConnectAccountRequest {
            account: Some(reference),
            backfill_history: false,
        };
        match accounts.connect_account(request).await {
            Ok(_) => {
                println!(
                    "[bench] connected {} ({})",
                    account.external_ref, account.uuid
                );
                connected += 1;
            }
            Err(status) => println!(
                "[bench] connect {} failed: {}",
                account.external_ref,
                status.message()
            ),
        }
    }
    Ok(connected)
}

/// This process's own PSS, so the report carries the client's cost instead of
/// leaving it out of the comparison.
fn self_pss_kib() -> Option<u64> {
    let rollup = std::fs::read_to_string("/proc/self/smaps_rollup").ok()?;
    rollup
        .lines()
        .find_map(|line| line.strip_prefix("Pss:"))
        .and_then(|value| value.trim().trim_end_matches(" kB").trim().parse().ok())
}

fn default_socket_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    format!("{home}/.local/state/wamux/wamux.sock")
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
