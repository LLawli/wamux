//! Sprint 3 scale gate (opt-in: `cargo test --test load_multi_account -- --ignored`).
//!
//! Synthetic, no real WhatsApp: register ~200 accounts and drive the live event
//! fan-out directly through each handle's broadcast. Proves the two properties
//! that matter at scale:
//!   1. No cross-account head-of-line blocking — a saturated/slow subscriber on
//!      one account does not stall delivery to a healthy subscriber on another.
//!   2. A slow subscriber gets a `gap` marker (lossy contract) instead of
//!      blocking the broadcast or growing unbounded.
//!
//! Requires the docker Postgres (DATABASE_URL).

use std::sync::Arc;
use std::time::Duration;

use tokio_stream::StreamExt;
use tonic::Request;

use wamux::proto::v1 as pb;
use wamux::proto::v1::event_service_server::EventService;
use wamux::services::event_service::EventSvc;
use wamux::state::{AccountRegistry, RegistryTuning};
use wamux::storage;

const ACCOUNTS: usize = 200;

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wamux:wamux@localhost:5433/wamux".into())
}

/// Delete leftover synthetic `load-%` accounts. Called at setup (self-heal from
/// an aborted prior run, whose best-effort teardown never ran) and at the end.
/// Setup-sweep bounds accumulation to at most one aborted run's rows (B5).
async fn sweep_load_orphans(pool: &sqlx::PgPool) -> u64 {
    sqlx::query("DELETE FROM accounts WHERE external_ref LIKE 'load-%'")
        .execute(pool)
        .await
        .map(|r| r.rows_affected())
        .unwrap_or(0)
}

fn subscribe_account(uuid: &str) -> pb::SubscribeRequest {
    pb::SubscribeRequest {
        selector: Some(pb::subscribe_request::Selector::Account(pb::AccountRef {
            r#ref: Some(pb::account_ref::Ref::Uuid(uuid.to_string())),
        })),
        replay_from_ring: 0,
    }
}

fn live_event(seq: i64) -> pb::EventEnvelope {
    pb::EventEnvelope {
        account_uuid: String::new(),
        monotonic_seq: seq,
        ts_unix_ms: 0,
        event: Some(pb::event_envelope::Event::Raw(pb::RawEvent {
            kind: "synthetic".to_string(),
            payload: vec![0u8; 64],
            note: String::new(),
        })),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "scale/stress test; run explicitly with --ignored"]
async fn no_head_of_line_blocking_and_gap_under_load() {
    let pool = storage::postgres::connect(&database_url(), 8)
        .await
        .expect("connect pg");
    storage::postgres::run_migrations(&pool)
        .await
        .expect("migrate");

    // Realistic broadcast: the modest probe burst fits cleanly, while the slow
    // account's heavy flood still overruns it and forces a lag → gap.
    let tuning = RegistryTuning {
        ring_capacity: 64,
        broadcast_capacity: 1024,
        ..RegistryTuning::default()
    };
    let registry = Arc::new(AccountRegistry::new(pool, tuning));

    // Self-heal before provisioning: clear any `load-%` rows an aborted prior run
    // left behind (the registry is fresh, so this doesn't touch live handles).
    let swept = sweep_load_orphans(registry.pool()).await;
    if swept > 0 {
        eprintln!("(swept {swept} orphan load- account(s) from a prior run)");
    }

    let tag = uuid::Uuid::new_v4();
    let mut handles = Vec::with_capacity(ACCOUNTS);
    for i in 0..ACCOUNTS {
        handles.push(
            registry
                .create_account(Some(&format!("load-{tag}-{i}")))
                .await
                .expect("create account"),
        );
    }
    assert!(registry.list().len() >= ACCOUNTS);
    assert_eq!(
        registry.connected_count(),
        0,
        "no real connections expected"
    );

    let svc = EventSvc::new(registry.clone());
    let slow_acct = handles[0].clone();
    let probe_acct = handles[ACCOUNTS - 1].clone();

    let mut slow = svc
        .subscribe_events(Request::new(subscribe_account(&slow_acct.uuid.to_string())))
        .await
        .expect("slow subscribe")
        .into_inner();
    let mut healthy = svc
        .subscribe_events(Request::new(subscribe_account(
            &probe_acct.uuid.to_string(),
        )))
        .await
        .expect("healthy subscribe")
        .into_inner();

    // Give the per-account forward tasks a moment to attach their receivers.
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Flood the slow account hard; nobody drains `slow` fast, so it must lag.
    let slow_tx = slow_acct.events_tx.clone();
    let flood = tokio::spawn(async move {
        for seq in 0..20_000i64 {
            let _ = slow_tx.send(live_event(seq));
            if seq % 1000 == 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
    });

    // Healthy probe on a DIFFERENT account: every event must arrive promptly even
    // while the slow account is saturated (no cross-account HOL blocking).
    const PROBE_EVENTS: i64 = 50;
    for seq in 0..PROBE_EVENTS {
        let _ = probe_acct.events_tx.send(live_event(seq));
    }
    let mut probe_received = 0i64;
    for _ in 0..PROBE_EVENTS {
        match tokio::time::timeout(Duration::from_secs(5), healthy.next()).await {
            Ok(Some(Ok(env))) => {
                assert_ne!(env.monotonic_seq, -1, "probe must not be gapped");
                probe_received += 1;
            }
            other => panic!("probe delivery stalled at {probe_received}: {other:?}"),
        }
    }
    assert_eq!(
        probe_received, PROBE_EVENTS,
        "healthy account delivered fully despite a saturated neighbor"
    );

    // Let the whole flood land before checking for the gap (B7): `broadcast::send`
    // never blocks, so this finishes fast and makes the lag deterministic — the
    // slow account's forwarder, blocked on its undrained onward channel, has now
    // definitely missed events, instead of racing the gap-check against the flood.
    flood.await.expect("flood task");

    // The slow subscriber, drained slowly, must eventually observe a gap marker.
    let mut saw_gap = false;
    for _ in 0..2000 {
        match tokio::time::timeout(Duration::from_millis(20), slow.next()).await {
            Ok(Some(Ok(env))) => {
                if env.monotonic_seq == -1 {
                    saw_gap = true;
                    break;
                }
            }
            Ok(_) => break,
            Err(_) => {} // read slowly: timeouts let the broadcast overflow behind us
        }
    }
    assert!(
        saw_gap,
        "slow subscriber should receive a gap marker under load"
    );

    // Tidy on success with a single sweep (not 200 sequential deletes that an
    // earlier panic could leave half-run). An aborted run is caught by the next
    // run's setup-sweep, so orphans never accumulate past one run (B5).
    let _ = sweep_load_orphans(registry.pool()).await;
}
