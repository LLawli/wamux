//! EventService: per-account / all-accounts event subscription with optional
//! ring replay. Delivery is backpressure-aware via an mpsc-backed stream.

use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::proto::v1 as pb;
use crate::proto::v1::event_service_server::EventService;
use crate::state::{AccountHandle, AccountRegistry};

pub struct EventSvc {
    registry: Arc<AccountRegistry>,
}

impl EventSvc {
    pub fn new(registry: Arc<AccountRegistry>) -> Self {
        Self { registry }
    }
}

/// Lossy-delivery contract: when a subscriber falls behind the broadcast buffer,
/// the core emits this marker instead of the dropped events. State transitions
/// can be among the dropped events, so the note carries the current connection
/// state and tells the edge to resync via `GetAccountStatus` (which reads the
/// authoritative watch channel).
fn gap_marker(account_uuid: &str, lagged: u64, state: pb::ConnectionState) -> pb::EventEnvelope {
    pb::EventEnvelope {
        account_uuid: account_uuid.to_string(),
        monotonic_seq: -1,
        ts_unix_ms: 0,
        event: Some(pb::event_envelope::Event::Raw(pb::RawEvent {
            kind: "gap".to_string(),
            payload: Vec::new(),
            note: format!("dropped {lagged} events; state={state:?}; resync via GetAccountStatus"),
        })),
    }
}

/// Gap marker for the account-created broadcast itself (all-accounts streams
/// only). Lagging here takes a burst of >64 creates while the follower is
/// starved; it means forwarders for some new accounts were never attached, so
/// their events are silently absent from this stream. The note hands the edge
/// the recovery primitives (re-subscribe or reconcile via `ListAccounts`) —
/// mechanism, not policy.
fn created_gap_marker(lagged: u64) -> pb::EventEnvelope {
    pb::EventEnvelope {
        account_uuid: String::new(),
        monotonic_seq: -1,
        ts_unix_ms: 0,
        event: Some(pb::event_envelope::Event::Raw(pb::RawEvent {
            kind: "gap".to_string(),
            payload: Vec::new(),
            note: format!(
                "dropped {lagged} account-created notifications; newly created \
                 accounts may be missing from this stream; re-subscribe or \
                 reconcile via ListAccounts"
            ),
        })),
    }
}

/// Forward one account's events (with optional ring replay) into `tx`.
fn forward(
    handle: Arc<AccountHandle>,
    replay: usize,
    tx: mpsc::Sender<Result<pb::EventEnvelope, Status>>,
) {
    tokio::spawn(async move {
        let mut rx = handle.subscribe();
        if replay > 0 {
            for envelope in handle.ring.snapshot(replay).await {
                if tx.send(Ok(envelope)).await.is_err() {
                    return;
                }
            }
        }
        let uuid = handle.uuid.to_string();
        loop {
            match rx.recv().await {
                Ok(envelope) => {
                    if tx.send(Ok(envelope)).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    let marker = gap_marker(&uuid, n, handle.current_state());
                    if tx.send(Ok(marker)).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

/// All-accounts subscription with dynamic membership (Sprint 5): forward every
/// account in the registry today AND every account created later. Contract
/// change: the created-follower task owns `tx`, so this stream now stays OPEN
/// indefinitely (before, it ended once the snapshot's forwarders ended). Also
/// documented on `SubscribeRequest.all_accounts` in proto/events.proto.
fn forward_all_accounts(
    registry: &AccountRegistry,
    replay: usize,
    tx: mpsc::Sender<Result<pb::EventEnvelope, Status>>,
) {
    // Order matters: subscribe to creations BEFORE snapshotting. An account
    // created between the two calls then shows up in both, and `seen` dedupes
    // it (two forwarders would duplicate its every event); the opposite order
    // would miss that account entirely.
    let created_rx = registry.subscribe_created();
    let snapshot = registry.list();
    let mut seen: HashSet<Uuid> = HashSet::with_capacity(snapshot.len());
    for account in snapshot {
        seen.insert(account.uuid);
        forward(account, replay, tx.clone());
    }
    follow_created_accounts(created_rx, seen, replay, tx);
}

/// Attach a forwarder for each account created after the subscribe. The same
/// `replay` is passed through: a brand-new account's ring is empty, so replay
/// is a harmless no-op for it.
fn follow_created_accounts(
    mut created_rx: broadcast::Receiver<Arc<AccountHandle>>,
    mut seen: HashSet<Uuid>,
    replay: usize,
    tx: mpsc::Sender<Result<pb::EventEnvelope, Status>>,
) {
    tokio::spawn(async move {
        while follow_created_step(&mut created_rx, &mut seen, replay, &tx).await {}
    });
}

/// One step of the created-follower loop; false = stop (client hung up or the
/// registry dropped).
async fn follow_created_step(
    created_rx: &mut broadcast::Receiver<Arc<AccountHandle>>,
    seen: &mut HashSet<Uuid>,
    replay: usize,
    tx: &mpsc::Sender<Result<pb::EventEnvelope, Status>>,
) -> bool {
    let received = tokio::select! {
        // Without this, an abandoned all-accounts stream would leak this task
        // (and `tx`) for the registry's whole lifetime: creates are rare, so
        // `recv()` alone might never wake up to notice the closed client.
        () = tx.closed() => return false,
        received = created_rx.recv() => received,
    };
    match received {
        Ok(account) => {
            // `seen` skips the subscribe/snapshot overlap (see forward_all_accounts).
            if seen.insert(account.uuid) {
                forward(account, replay, tx.clone());
            }
            true
        }
        Err(RecvError::Lagged(missed)) => tx.send(Ok(created_gap_marker(missed))).await.is_ok(),
        Err(RecvError::Closed) => false,
    }
}

#[tonic::async_trait]
impl EventService for EventSvc {
    type SubscribeEventsStream = ReceiverStream<Result<pb::EventEnvelope, Status>>;

    async fn subscribe_events(
        &self,
        request: Request<pb::SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeEventsStream>, Status> {
        let req = request.into_inner();
        let replay = req.replay_from_ring as usize;
        let (tx, rx) = mpsc::channel(256);

        match req.selector {
            Some(pb::subscribe_request::Selector::Account(account_ref)) => {
                let handle = self.registry.resolve(Some(&account_ref))?;
                forward(handle, replay, tx);
            }
            Some(pb::subscribe_request::Selector::AllAccounts(_)) => {
                forward_all_accounts(&self.registry, replay, tx);
            }
            None => {
                // No selector => send-only client; empty (immediately-closed) stream.
                drop(tx);
            }
        }

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gap_marker_carries_count_and_state() {
        let marker = gap_marker("acc-1", 42, pb::ConnectionState::Connected);
        assert_eq!(marker.monotonic_seq, -1);
        let Some(pb::event_envelope::Event::Raw(raw)) = marker.event else {
            panic!("gap marker must be a raw event");
        };
        assert_eq!(raw.kind, "gap");
        assert!(raw.note.contains("dropped 42 events"));
        assert!(raw.note.contains("state=Connected"));
        assert!(raw.note.contains("GetAccountStatus"));
    }

    #[test]
    fn created_gap_marker_points_edge_at_list_accounts() {
        let marker = created_gap_marker(7);
        assert_eq!(marker.monotonic_seq, -1);
        assert!(marker.account_uuid.is_empty(), "not tied to one account");
        let Some(pb::event_envelope::Event::Raw(raw)) = marker.event else {
            panic!("created gap marker must be a raw event");
        };
        assert_eq!(raw.kind, "gap");
        assert!(raw.note.contains("dropped 7 account-created"));
        assert!(raw.note.contains("re-subscribe"));
        assert!(raw.note.contains("ListAccounts"));
    }
}
