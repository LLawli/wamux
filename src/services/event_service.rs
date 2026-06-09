//! EventService: per-account / all-accounts event subscription with optional
//! ring replay. Delivery is backpressure-aware via an mpsc-backed stream.

use std::sync::Arc;

use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

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
                for handle in self.registry.list() {
                    forward(handle, replay, tx.clone());
                }
                drop(tx); // close when all per-account forwarders end
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
}
