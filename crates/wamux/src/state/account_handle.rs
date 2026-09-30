//! Per-account runtime handle: holds the live `Client`, the running bot, the
//! event broadcast channel, the replay ring, and the connection-state watch.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use tokio::sync::{Mutex, RwLock, broadcast, watch};
use tokio::task::JoinHandle;
use uuid::Uuid;
use whatsapp_rust::Client;

use super::event_ring::EventRing;
use crate::proto::v1::{ConnectionState, EventEnvelope};
use crate::storage::AccountRow;

/// The supervisor task for a running bot: it owns the `Bot`, awaits the run
/// loop's terminal exit (the library handles transient reconnects internally,
/// so this resolves only when the library has given up), then marks the account
/// down truthfully. Aborting it is the hard-stop fallback.
pub struct RunningBot {
    pub supervisor: JoinHandle<()>,
}

pub struct AccountHandle {
    pub uuid: Uuid,
    pub external_ref: Option<String>,
    pub device_id: i32,
    pub push_name: Option<String>,
    /// Live event fan-out to subscribers.
    pub events_tx: broadcast::Sender<EventEnvelope>,
    /// Short replay buffer for quick reconnects.
    pub ring: Arc<EventRing>,
    /// Current connection state (writer shared into the event bridge).
    pub state_tx: Arc<watch::Sender<ConnectionState>>,
    pub state_rx: watch::Receiver<ConnectionState>,
    /// Monotonic per-account event sequence.
    pub seq: Arc<AtomicI64>,
    /// Grace period for `stop` before falling back to detaching the run loop.
    graceful_stop_timeout: Duration,
    client: RwLock<Option<Arc<Client>>>,
    running: Mutex<Option<RunningBot>>,
}

impl AccountHandle {
    pub fn new(
        row: &AccountRow,
        ring_capacity: usize,
        broadcast_capacity: usize,
        graceful_stop_timeout: Duration,
    ) -> Self {
        let (events_tx, _) = broadcast::channel(broadcast_capacity);
        let (state_tx, state_rx) = watch::channel(ConnectionState::Disconnected);
        Self {
            uuid: row.uuid,
            external_ref: row.external_ref.clone(),
            device_id: row.device_id,
            push_name: row.push_name.clone(),
            events_tx,
            ring: Arc::new(EventRing::new(ring_capacity)),
            state_tx: Arc::new(state_tx),
            state_rx,
            seq: Arc::new(AtomicI64::new(0)),
            graceful_stop_timeout,
            client: RwLock::new(None),
            running: Mutex::new(None),
        }
    }

    pub fn next_seq(&self) -> i64 {
        self.seq.fetch_add(1, Ordering::Relaxed)
    }

    pub async fn set_client(&self, client: Arc<Client>) {
        *self.client.write().await = Some(client);
    }

    pub async fn client(&self) -> Option<Arc<Client>> {
        self.client.read().await.clone()
    }

    pub async fn set_running(&self, running: RunningBot) {
        // Defensive only: `connect` guards on `is_running`, so a live replace
        // shouldn't happen. Aborting the previous supervisor would orphan its
        // run loop, hence the guard upstream is what we rely on.
        if let Some(previous) = self.running.lock().await.replace(running) {
            previous.supervisor.abort();
        }
    }

    pub async fn is_running(&self) -> bool {
        self.running.lock().await.is_some()
    }

    /// Called by the supervisor when the run loop has terminally exited (the
    /// library stopped reconnecting). Clears the running slot + client and makes
    /// the connection state truthful, preserving a terminal state already set by
    /// an event (LoggedOut/Banned). Idempotent.
    pub async fn on_bot_exited(&self) {
        self.running.lock().await.take();
        *self.client.write().await = None;
        let terminal = matches!(
            self.current_state(),
            ConnectionState::LoggedOut | ConnectionState::Banned
        );
        if !terminal {
            let _ = self.state_tx.send(ConnectionState::Disconnected);
        }
    }

    /// Graceful stop: ask the library to exit its run loop (`disconnect` flushes
    /// outbound + closes the transport, flipping `is_running` off), then wait for
    /// the supervisor to observe the exit. On timeout we detach rather than abort
    /// — `disconnect` already requested the exit, so the run loop self-terminates
    /// and no task is orphaned.
    pub async fn stop(&self) {
        let client = self.client().await;
        let supervisor = self.running.lock().await.take().map(|r| r.supervisor);

        if let Some(client) = &client {
            client.disconnect().await;
        }
        if let Some(supervisor) = supervisor
            && tokio::time::timeout(self.graceful_stop_timeout, supervisor)
                .await
                .is_err()
        {
            tracing::warn!(
                uuid = %self.uuid,
                "graceful stop timed out; run loop will exit on its own"
            );
        }

        *self.client.write().await = None;
        let _ = self.state_tx.send(ConnectionState::Disconnected);
    }

    pub fn current_state(&self) -> ConnectionState {
        *self.state_rx.borrow()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.events_tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle() -> AccountHandle {
        let row = AccountRow {
            uuid: Uuid::new_v4(),
            external_ref: None,
            device_id: 1,
            push_name: None,
            created_at: 0,
        };
        AccountHandle::new(&row, 8, 16, Duration::from_millis(10))
    }

    #[tokio::test]
    async fn on_bot_exited_makes_state_disconnected() {
        let h = handle();
        let _ = h.state_tx.send(ConnectionState::Connected);
        h.on_bot_exited().await;
        assert_eq!(h.current_state(), ConnectionState::Disconnected);
        assert!(!h.is_running().await);
    }

    #[tokio::test]
    async fn on_bot_exited_preserves_terminal_state() {
        let h = handle();
        let _ = h.state_tx.send(ConnectionState::LoggedOut);
        h.on_bot_exited().await;
        // A terminal state set by an event must survive the run-loop teardown.
        assert_eq!(h.current_state(), ConnectionState::LoggedOut);
    }
}
