//! Steady-state message load with many accounts (#218): the measurement the
//! allocator choice rests on. The handshake test (`stress_handshake`) is bound
//! by Noise and store I/O; this one keeps N logged-in companions sending texts
//! through the socket, so each message pays a Signal ratchet step, protobuf
//! encode, a stanza, a store write and a gRPC round trip.
//!
//! Opt-in, release only:
//! `cargo test --release --features stress --test stress_message_load -- --ignored --nocapture`
//! Env: `STRESS_ACCOUNTS` (default 100), `STRESS_MESSAGES` per account after the
//! warm-up (default 100), plus the usual `WAMUX_TEST_ENGINE`/`DATABASE_URL`.
//!
//! The report line covers the measured phase only: the first message of each
//! account builds its Signal session (a `pkmsg`) and is left out, so what is
//! timed is the steady state. CPU comes from `/proc/self/stat` (user + system
//! of the whole process), peak RSS from `VmHWM`.
#![cfg(feature = "stress")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use tonic::transport::Channel;
use wacore::store::Device;
use wamux::proto::v1 as pb;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;
use wamux::state::{AccountHandle, AccountRegistry, RegistryTuning};
use wamux::stress::{MockIdentity, MockPeer, MockWaServer};

// The allocator `main.rs` gives the shipped daemon on musl: a test binary does
// not inherit it, and a musl run of this harness has to measure what ships.
#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

// Only a subset of the shared helpers is used per test binary.
#[allow(dead_code)]
mod common;

const PEER_PN_USER: &str = "5511900000002";
const PEER_LID_USER: &str = "100000000000002";
const OWN_PN_USER: &str = "5511999999999";
const OWN_LID_USER: &str = "100000000000099";
const OWN_LID: &str = "100000000000099:7@lid";
/// Linux reports `/proc/self/stat` times in USER_HZ, fixed at 100 on x86_64.
const CLOCK_TICKS_PER_SEC: f64 = 100.0;

fn env_count(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// User + system CPU seconds this process has used so far.
fn cpu_seconds() -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").expect("read /proc/self/stat");
    // The command name can hold spaces; the fields after its `)` are fixed.
    let after_comm = &stat[stat.rfind(')').expect("comm field") + 2..];
    let fields: Vec<&str> = after_comm.split_whitespace().collect();
    let utime: f64 = fields[11].parse().expect("utime");
    let stime: f64 = fields[12].parse().expect("stime");
    (utime + stime) / CLOCK_TICKS_PER_SEC
}

/// Peak resident set size of this process, in MiB.
fn peak_rss_mib() -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").expect("read /proc/self/status");
    let line = status
        .lines()
        .find(|l| l.starts_with("VmHWM:"))
        .expect("VmHWM line");
    let kib: f64 = line
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .expect("VmHWM value");
    kib / 1024.0
}

/// N companions on one registry, each the same paired identity the
/// MessagingService suite uses, each with its own store rows.
async fn provision(registry: &AccountRegistry, prefix: &str, n: usize) -> Vec<Arc<AccountHandle>> {
    let tag = uuid::Uuid::new_v4();
    let mut handles = Vec::with_capacity(n);
    for i in 0..n {
        let handle = registry
            .create_account(Some(&format!("{prefix}{tag}-{i}")))
            .await
            .expect("create account");
        let mut device = Device::new();
        device.pn = Some(common::COMPANION_PN.parse().expect("companion pn"));
        device.push_name = "Load".to_string();
        device.account = Some(Arc::new(common::zeroed_adv_identity()));
        registry
            .storage()
            .device_backend(handle.device_id)
            .save(&device)
            .await
            .expect("save companion device");
        handles.push(handle);
    }
    handles
}

/// Connect every account and wait until each one is logged in with its LID.
async fn log_in_all(registry: &AccountRegistry, handles: &[Arc<AccountHandle>]) {
    for handle in handles {
        registry.connect(handle, None, true).await.expect("connect");
    }
    for handle in handles {
        let client = handle.client().await.expect("client after connect");
        common::poll_until("a client to log in", Duration::from_secs(60), || async {
            (client.is_logged_in() && client.lid().is_some()).then_some(())
        })
        .await;
    }
}

fn account_ref(handle: &AccountHandle) -> pb::AccountRef {
    pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::Uuid(handle.uuid.to_string())),
    }
}

/// `count` texts from one account to the peer, one after the other.
async fn send_texts(channel: Channel, account: pb::AccountRef, peer: String, count: usize) {
    let mut client = MessagingServiceClient::new(channel);
    for i in 0..count {
        let request = pb::SendTextRequest {
            account: Some(account.clone()),
            to: Some(pb::Jid {
                value: peer.clone(),
            }),
            text: format!("load message {i}: steady state traffic for the allocator measurement"),
            ..Default::default()
        };
        client.send_text(request).await.expect("send text");
    }
}

/// Every account sends `count` texts at once; returns when all are answered.
async fn send_round(channel: &Channel, handles: &[Arc<AccountHandle>], peer: &str, count: usize) {
    let mut tasks = tokio::task::JoinSet::new();
    for handle in handles {
        tasks.spawn(send_texts(
            channel.clone(),
            account_ref(handle),
            peer.to_string(),
            count,
        ));
    }
    while let Some(joined) = tasks.join_next().await {
        joined.expect("sender task");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "load measurement; run explicitly with --release --ignored"]
async fn steady_state_message_load_with_many_accounts() {
    let accounts = env_count("STRESS_ACCOUNTS", 100);
    let messages = env_count("STRESS_MESSAGES", 100);

    let mock = MockWaServer::start_as(MockIdentity {
        lid: OWN_LID.to_string(),
    })
    .await
    .expect("start mock");
    let peer = MockPeer::new(PEER_PN_USER, PEER_LID_USER)
        .await
        .expect("peer");
    let phone = MockPeer::new(OWN_PN_USER, OWN_LID_USER)
        .await
        .expect("own phone");
    mock.serve_peer(&peer);
    mock.serve_peer(&phone);

    let engine = common::test_engine_with(16).await;
    let prefix = common::test_prefix("stress_message_load", "steady_state");
    common::sweep_orphans(&engine, &prefix).await;
    let tuning = RegistryTuning {
        ws_url_override: Some(mock.ws_url()),
        graceful_stop_timeout: Duration::from_millis(500),
        ..RegistryTuning::default()
    };
    let registry = Arc::new(AccountRegistry::new(engine, tuning));
    let handles = provision(&registry, &prefix, accounts).await;
    log_in_all(&registry, &handles).await;
    let channel = common::serve_registry(registry.clone()).await;
    let peer_jid = peer.pn().to_string();

    // Warm-up: the first send of each account builds its Signal session.
    send_round(&channel, &handles, &peer_jid, 1).await;

    let cpu_before = cpu_seconds();
    let started = Instant::now();
    send_round(&channel, &handles, &peer_jid, messages).await;
    // The RPC answers once the client wrote the frame, a beat before the mock
    // reads it, so the clock stops when the last stanza has landed.
    let expected = accounts * (messages + 1);
    common::poll_until(
        "every stanza to reach the mock",
        Duration::from_secs(120),
        || async { (mock.client_message_count() >= expected).then_some(()) },
    )
    .await;
    let wall = started.elapsed().as_secs_f64();
    let cpu = cpu_seconds() - cpu_before;

    let total = accounts * messages;
    let sent_to_mock = mock.client_message_count();
    println!(
        "LOAD accounts={accounts} messages={total} wall_s={wall:.2} msgs_per_s={:.0} \
         cpu_s={cpu:.2} cpu_us_per_msg={:.0} peak_rss_mib={:.0} stanzas={sent_to_mock}",
        total as f64 / wall,
        cpu * 1e6 / total as f64,
        peak_rss_mib(),
    );

    for handle in &handles {
        let _ = registry.delete(handle).await;
    }
    common::sweep_orphans(registry.storage(), &prefix).await;
}
