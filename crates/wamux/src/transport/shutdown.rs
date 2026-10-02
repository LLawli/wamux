//! Graceful shutdown: the OS signal, the one-shot `Shutdown` every long-lived
//! stream watches, and the bounded wait for the server to drain.
//!
//! Why all three (#35): tonic answers the shutdown signal by waiting, with no
//! deadline, for every connection to close, and HTTP/2 lets in-flight streams
//! finish. A `SubscribeEvents` stream never finishes on its own, so SIGTERM
//! used to park the daemon until systemd SIGKILLed it. Now the event forwarders
//! end their streams when `Shutdown` fires, and `drain_or_deadline` caps
//! whatever else might still hold a connection.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::watch;

/// Fires once, observed by any number of tasks. A `watch` rather than a
/// `Notify`: a task that starts waiting AFTER the trigger must still see it
/// (a subscription opened during the drain has to end too).
#[derive(Clone)]
pub struct Shutdown {
    tx: Arc<watch::Sender<bool>>,
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::new()
    }
}

impl Shutdown {
    pub fn new() -> Self {
        Self {
            tx: Arc::new(watch::channel(false).0),
        }
    }

    /// Idempotent: a second trigger is a no-op.
    pub fn trigger(&self) {
        self.tx.send_replace(true);
    }

    pub fn is_triggered(&self) -> bool {
        *self.tx.borrow()
    }

    /// Resolves once triggered, immediately if that already happened. `self`
    /// holds the sender, so `wait_for` can never see it dropped.
    pub async fn triggered(&self) {
        let mut rx = self.tx.subscribe();
        let _ = rx.wait_for(|fired| *fired).await;
    }
}

/// Run `serve` to completion, but once `shutdown` fires give it at most
/// `grace` more. `None` means the grace elapsed and `serve` was dropped, which
/// closes whatever connections it still held.
pub async fn drain_or_deadline<F: Future>(
    serve: F,
    shutdown: &Shutdown,
    grace: Duration,
) -> Option<F::Output> {
    tokio::select! {
        output = serve => Some(output),
        () = async {
            shutdown.triggered().await;
            tokio::time::sleep(grace).await;
        } => None,
    }
}

/// Resolves on SIGTERM or SIGINT.
pub async fn signal_future() {
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "failed to install SIGTERM handler");
            return;
        }
    };
    let mut interrupt = match signal(SignalKind::interrupt()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "failed to install SIGINT handler");
            return;
        }
    };
    tokio::select! {
        _ = term.recv() => tracing::info!("received SIGTERM, shutting down"),
        _ = interrupt.recv() => tracing::info!("received SIGINT, shutting down"),
    }
}

#[cfg(test)]
mod tests;
