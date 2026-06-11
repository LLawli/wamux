//! Run a non-Send library future to completion on a dedicated current-thread
//! runtime (inside `spawn_blocking`). Several whatsapp-rust queries (usync,
//! participating groups) return futures that aren't `Send` (HRTB), so they can't
//! live inside an `#[async_trait]` future. Only the owned `T` crosses back.

use crate::error::{WamuxError, client_err};

pub(crate) async fn run_isolated<F, Fut, T, E>(make: F) -> Result<T, WamuxError>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, E>>,
    T: Send + 'static,
    E: Into<anyhow::Error>,
{
    tokio::task::spawn_blocking(move || {
        // Everything funnels through the shared classifier: the library error
        // keeps its honest IQ Status code, and the infra errors (runtime
        // build, join) just fall through to the opaque Client arm.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(client_err)?;
        runtime.block_on(make()).map_err(client_err)
    })
    .await
    .map_err(client_err)?
}
