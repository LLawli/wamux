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
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| WamuxError::Client(e.to_string()))?;
        // The library error goes through the shared classifier so IQ server
        // rejections keep their honest Status code on this path too.
        runtime.block_on(make()).map_err(client_err)
    })
    .await
    .map_err(|e| WamuxError::Client(e.to_string()))?
}
