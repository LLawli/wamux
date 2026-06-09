//! A future that resolves on SIGTERM or SIGINT, used to drive graceful shutdown.

use tokio::signal::unix::{SignalKind, signal};

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
