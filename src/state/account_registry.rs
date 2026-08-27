//! The single shared `Arc<AccountRegistry>` injected into every service: owns
//! the storage engine (whichever backend the DSN selected) and all per-account
//! handles. Nothing here is engine-aware: account persistence and the
//! per-account `Backend` both come from `StorageEngine`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use tokio::sync::broadcast;
use uuid::Uuid;
use whatsapp_rust::pair_code::PairCodeOptions;

use crate::domain::bot_factory::build_bot;
use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;
use crate::state::account_handle::RunningBot;
use crate::state::event_bridge::EventCtx;
use crate::state::{AccountHandle, EventRing};
use crate::storage::{AccountRow, StorageEngine};

/// Deployment-tunable runtime knobs, injected once from `Config`. Bins/tests use
/// `Default` and override only what they care about.
#[derive(Debug, Clone)]
pub struct RegistryTuning {
    pub ring_capacity: usize,
    pub broadcast_capacity: usize,
    pub replay_max_event_bytes: u64,
    pub max_connected_accounts: usize,
    pub graceful_stop_timeout: Duration,
    /// Override the upstream WebSocket URL (e.g. `ws://127.0.0.1:PORT` for the
    /// stress mock). `None` = the real WhatsApp endpoint. Transport injection,
    /// not policy.
    pub ws_url_override: Option<String>,
}

impl Default for RegistryTuning {
    fn default() -> Self {
        Self {
            ring_capacity: 256,
            broadcast_capacity: 1024,
            replay_max_event_bytes: 0,
            max_connected_accounts: 0,
            graceful_stop_timeout: Duration::from_millis(3000),
            ws_url_override: None,
        }
    }
}

impl RegistryTuning {
    /// Convenience for bins/tests that only care about the ring size.
    pub fn with_ring(ring_capacity: usize) -> Self {
        Self {
            ring_capacity,
            ..Self::default()
        }
    }
}

/// Decide whether one more connection fits the budget. `cap == 0` is unlimited.
fn within_connection_budget(current: usize, cap: usize) -> bool {
    cap == 0 || current < cap
}

pub struct AccountRegistry {
    storage: Arc<dyn StorageEngine>,
    tuning: RegistryTuning,
    /// Count of accounts with a live supervisor (incremented on a successful
    /// connect, decremented once when the run loop exits). Single-owner so it
    /// can't double-count across stop vs. terminal exit.
    connected: Arc<AtomicUsize>,
    handles: DashMap<Uuid, Arc<AccountHandle>>,
    by_external: DashMap<String, Uuid>,
    /// Fan-out of newly inserted handles so all-accounts event subscriptions can
    /// attach a forwarder dynamically. Capacity 64 is plenty: account creation
    /// is a rare, edge-driven action, never a hot path.
    created_tx: broadcast::Sender<Arc<AccountHandle>>,
}

impl AccountRegistry {
    pub fn new(storage: Arc<dyn StorageEngine>, tuning: RegistryTuning) -> Self {
        let (created_tx, _) = broadcast::channel(64);
        Self {
            storage,
            tuning,
            connected: Arc::new(AtomicUsize::new(0)),
            handles: DashMap::new(),
            by_external: DashMap::new(),
            created_tx,
        }
    }

    /// The storage engine backing this registry (account CRUD + `Backend`
    /// factory). Services use it instead of reaching for a driver-specific pool.
    pub fn storage(&self) -> &Arc<dyn StorageEngine> {
        &self.storage
    }

    fn insert_handle(&self, row: &AccountRow) -> Arc<AccountHandle> {
        let handle = Arc::new(AccountHandle::new(
            row,
            self.tuning.ring_capacity,
            self.tuning.broadcast_capacity,
            self.tuning.graceful_stop_timeout,
        ));
        if let Some(external) = &handle.external_ref {
            self.by_external.insert(external.clone(), handle.uuid);
        }
        self.handles.insert(handle.uuid, handle.clone());
        // Notify AFTER registering, so a receiver can immediately resolve the
        // handle. No receivers is the normal case (no all-accounts subscriber).
        let _ = self.created_tx.send(handle.clone());
        handle
    }

    /// Load all persisted accounts into handles. Does NOT connect: connect is
    /// entirely edge-driven (the edge calls `Connect` for the accounts it wants
    /// live, including after a daemon restart). No always-on policy in the core.
    pub async fn load_existing(self: &Arc<Self>) -> Result<(), WamuxError> {
        for row in self.storage.list_accounts().await? {
            self.insert_handle(&row);
        }
        Ok(())
    }

    pub async fn create_account(
        &self,
        external_ref: Option<&str>,
    ) -> Result<Arc<AccountHandle>, WamuxError> {
        let row = self.storage.create_account(external_ref).await?;
        Ok(self.insert_handle(&row))
    }

    pub fn get(&self, uuid: &Uuid) -> Option<Arc<AccountHandle>> {
        self.handles.get(uuid).map(|h| h.clone())
    }

    pub fn list(&self) -> Vec<Arc<AccountHandle>> {
        self.handles.iter().map(|h| h.clone()).collect()
    }

    /// Watch handles inserted after this call. All-accounts event subscriptions
    /// combine this with `list()` to get dynamic membership (Sprint 5).
    pub fn subscribe_created(&self) -> broadcast::Receiver<Arc<AccountHandle>> {
        self.created_tx.subscribe()
    }

    /// Resolve a proto `AccountRef` (uuid or external_ref) to a handle.
    pub fn resolve(
        &self,
        account_ref: Option<&pb::AccountRef>,
    ) -> Result<Arc<AccountHandle>, WamuxError> {
        let reference = account_ref
            .and_then(|r| r.r#ref.as_ref())
            .ok_or_else(|| WamuxError::InvalidArgument("missing account ref".to_string()))?;
        match reference {
            pb::account_ref::Ref::Uuid(uuid) => {
                let id = Uuid::parse_str(uuid)
                    .map_err(|_| WamuxError::InvalidArgument(format!("bad uuid '{uuid}'")))?;
                self.get(&id)
                    .ok_or_else(|| WamuxError::AccountNotFound(uuid.clone()))
            }
            pb::account_ref::Ref::ExternalRef(external) => {
                let id = self
                    .by_external
                    .get(external)
                    .map(|v| *v)
                    .ok_or_else(|| WamuxError::AccountNotFound(external.clone()))?;
                self.get(&id)
                    .ok_or_else(|| WamuxError::AccountNotFound(external.clone()))
            }
        }
    }

    /// Build + run the bot for an account (idempotent if already running).
    ///
    /// `skip_history` is the relay-pure default (true). Pass `false` (via
    /// `ConnectAccountRequest.backfill_history`) to let the phone's history dump
    /// flow through as `HistorySyncEvent`s. Takes effect at build time, so a
    /// no-op when the account is already running.
    pub async fn connect(
        &self,
        handle: &Arc<AccountHandle>,
        pair_code: Option<PairCodeOptions>,
        skip_history: bool,
    ) -> Result<(), WamuxError> {
        if handle.is_running().await {
            return Ok(());
        }
        // fd/resource budget (soft): a small check-then-increment TOCTOU window is
        // acceptable for a self-protection cap. The edge owns connect policy.
        if !within_connection_budget(
            self.connected.load(Ordering::SeqCst),
            self.tuning.max_connected_accounts,
        ) {
            return Err(WamuxError::ResourceExhausted(format!(
                "connection budget reached ({})",
                self.tuning.max_connected_accounts
            )));
        }
        let _ = handle.state_tx.send(pb::ConnectionState::Connecting);
        let ctx = EventCtx {
            uuid: handle.uuid.to_string(),
            events_tx: handle.events_tx.clone(),
            ring: handle.ring.clone(),
            state_tx: handle.state_tx.clone(),
            seq: handle.seq.clone(),
            replay_max_event_bytes: self.tuning.replay_max_event_bytes,
        };
        let mut bot = build_bot(
            self.storage.device_backend(handle.device_id),
            ctx,
            pair_code,
            skip_history,
            self.tuning.ws_url_override.as_deref(),
        )
        .await
        .map_err(client_err)?;
        handle.set_client(bot.client()).await;
        let bot_handle = bot.run().await.map_err(client_err)?;

        self.connected.fetch_add(1, Ordering::SeqCst);
        // Supervisor: own the Bot for the connection's lifetime and await the run
        // loop's terminal exit (the library reconnects transient drops itself, so
        // this resolves only when it has given up). Then mark the account down and
        // release the budget slot — exactly once per connect.
        let supervised = handle.clone();
        let connected = self.connected.clone();
        let supervisor = tokio::spawn(async move {
            let _bot = bot;
            let _ = bot_handle.await;
            supervised.on_bot_exited().await;
            connected.fetch_sub(1, Ordering::SeqCst);
        });
        handle.set_running(RunningBot { supervisor }).await;
        Ok(())
    }

    pub async fn disconnect(&self, handle: &Arc<AccountHandle>) {
        handle.stop().await;
    }

    /// Real device unlink: send the server-side RemoveCompanionDevice IQ, then
    /// stop the bot. Keeps the `accounts` row + local keys so the account can be
    /// re-paired (use `delete` to wipe state). The library only emits the IQ
    /// while connected, so we require a live connection and surface `NotConnected`
    /// otherwise — the edge decides whether to connect first.
    pub async fn logout(&self, handle: &Arc<AccountHandle>) -> Result<(), WamuxError> {
        let client = handle.client().await.ok_or(WamuxError::NotConnected)?;
        if !client.is_connected() {
            return Err(WamuxError::NotConnected);
        }
        client.logout().await.map_err(client_err)?;
        handle.stop().await;
        Ok(())
    }

    /// Stop the bot and delete the account (cascade removes all scoped rows).
    pub async fn delete(&self, handle: &Arc<AccountHandle>) -> Result<(), WamuxError> {
        handle.stop().await;
        self.storage.delete_account(handle.uuid).await?;
        if let Some(external) = &handle.external_ref {
            self.by_external.remove(external);
        }
        self.handles.remove(&handle.uuid);
        Ok(())
    }

    pub fn ring_capacity(&self) -> usize {
        self.tuning.ring_capacity
    }

    /// Accounts currently held connected (live supervisor). For metrics/tests.
    pub fn connected_count(&self) -> usize {
        self.connected.load(Ordering::SeqCst)
    }
}

/// Re-export for callers that build handles directly in tests.
pub use crate::state::account_handle::AccountHandle as Handle;
pub type Ring = EventRing;

#[cfg(test)]
mod tests {
    use super::within_connection_budget;

    #[test]
    fn budget_zero_is_unlimited() {
        assert!(within_connection_budget(0, 0));
        assert!(within_connection_budget(10_000, 0));
    }

    #[test]
    fn budget_rejects_at_or_above_cap() {
        assert!(within_connection_budget(0, 2));
        assert!(within_connection_budget(1, 2));
        assert!(!within_connection_budget(2, 2));
        assert!(!within_connection_budget(3, 2));
    }
}
