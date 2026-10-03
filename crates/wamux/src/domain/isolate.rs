//! Run a non-Send library future to completion on a blocking thread, driven by
//! the caller's own tokio runtime (`Handle::block_on` inside `spawn_blocking`).
//! Several whatsapp-rust queries (usync, participating groups) return futures
//! that aren't `Send` (HRTB), so they can't live inside an `#[async_trait]`
//! future. Only the owned `T` crosses back.
//!
//! The future must NOT get a runtime of its own (#107): connections the Postgres
//! pool opens inside it, timers, and tasks the library `tokio::spawn`s would all
//! die with a throwaway runtime, leaving every later pool acquire stuck until
//! `acquire_timeout`.

use crate::error::{WamuxError, client_err};

/// Public only so the integration tests can drive it (#107); not part of the
/// crate's API.
#[doc(hidden)]
pub async fn run_isolated<F, Fut, T, E>(make: F) -> Result<T, WamuxError>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, E>>,
    T: Send + 'static,
    E: Into<anyhow::Error>,
{
    // `run_isolated` is async and only ever runs inside the daemon's runtime, so
    // `Handle::current()` cannot be called outside a tokio context here.
    let handle: tokio::runtime::Handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        // `Handle::block_on` does not drive the I/O and time drivers itself on a
        // current_thread runtime (only `Runtime::block_on` does). That is fine
        // because the daemon is multi-thread (`#[tokio::main]` in main.rs), so
        // its worker threads keep the drivers moving while this blocking thread
        // waits (#107).
        //
        // Everything funnels through the shared classifier: the library error
        // keeps its honest IQ Status code, and the infra error (join) just
        // falls through to the opaque Client arm.
        handle.block_on(make()).map_err(client_err)
    })
    .await
    .map_err(client_err)?
}

#[cfg(test)]
#[path = "isolate_tests.rs"]
mod tests;
