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
mod shutdown_tests {
    use super::*;

    const SOON: Duration = Duration::from_millis(200);

    #[tokio::test]
    async fn a_waiter_wakes_when_shutdown_fires() {
        let shutdown = Shutdown::new();
        let waiter = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.triggered().await }
        });
        assert!(!shutdown.is_triggered());
        shutdown.trigger();
        tokio::time::timeout(SOON, waiter).await.unwrap().unwrap();
    }

    // A stream opened during the drain must end too, so a late waiter cannot
    // be allowed to miss a trigger that already happened.
    #[tokio::test]
    async fn a_waiter_that_arrives_late_still_sees_the_trigger() {
        let shutdown = Shutdown::new();
        shutdown.trigger();
        shutdown.trigger();
        tokio::time::timeout(SOON, shutdown.triggered())
            .await
            .expect("an already-fired shutdown must resolve at once");
    }

    #[tokio::test]
    async fn a_serve_that_finishes_returns_its_output() {
        let shutdown = Shutdown::new();
        let out = drain_or_deadline(async { 7 }, &shutdown, Duration::from_secs(60)).await;
        assert_eq!(out, Some(7));
    }

    // The backstop: something that never drains is dropped after the grace,
    // and not before shutdown fires.
    #[tokio::test]
    async fn a_serve_that_never_drains_is_cut_off_after_the_grace() {
        let shutdown = Shutdown::new();
        shutdown.trigger();
        let out = tokio::time::timeout(
            SOON * 5,
            drain_or_deadline(std::future::pending::<()>(), &shutdown, SOON),
        )
        .await
        .expect("the grace must bound the wait");
        assert_eq!(out, None);
    }

    #[tokio::test]
    async fn the_grace_only_starts_counting_at_the_trigger() {
        let shutdown = Shutdown::new();
        let pending = drain_or_deadline(std::future::pending::<()>(), &shutdown, SOON);
        assert!(
            tokio::time::timeout(SOON * 3, pending).await.is_err(),
            "without a trigger the server must keep serving"
        );
    }
}
