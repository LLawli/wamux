//! Build a `whatsapp_rust::Bot` for one account, wired to the Postgres backend,
//! tokio transport/runtime, ureq HTTP, and the wamux event bridge.

use std::sync::Arc;

use sqlx::PgPool;
use whatsapp_rust::TokioRuntime;
use whatsapp_rust::bot::Bot;
use whatsapp_rust::pair_code::PairCodeOptions;
use whatsapp_rust::store::traits::Backend;
use whatsapp_rust_tokio_transport::{Connector, TokioWebSocketTransportFactory};
use whatsapp_rust_ureq_http_client::UreqHttpClient;

use crate::state::event_bridge::{self, EventCtx};
use crate::storage::postgres::PgBackend;

/// Construct (but do not run) a Bot for `device_id`. Pass `pair_code` to start
/// phone-number pairing on connect; omit it to pair via QR (emitted as events).
///
/// `skip_history`: relay-pure default. When `true` the phone's history dump is
/// dropped (receipt sent so it stops retrying); when `false` it is processed and
/// emitted as `HistorySyncEvent`s for the edge to backfill (e.g. a CRM).
pub async fn build_bot(
    pool: PgPool,
    device_id: i32,
    ctx: EventCtx,
    pair_code: Option<PairCodeOptions>,
    skip_history: bool,
    ws_url: Option<&str>,
) -> anyhow::Result<Bot> {
    let backend: Arc<dyn Backend> = Arc::new(PgBackend::new(pool, device_id));

    // `ws_url` overrides the upstream endpoint (stress mock); plaintext ws://
    // needs the Plain connector since the default forces TLS.
    let transport = match ws_url {
        Some(url) => TokioWebSocketTransportFactory::new()
            .with_url(url)
            .with_connector(Connector::Plain),
        None => TokioWebSocketTransportFactory::new(),
    };

    let mut builder = Bot::builder()
        .with_backend(backend)
        .with_transport_factory(transport)
        .with_http_client(UreqHttpClient::new())
        .with_runtime(TokioRuntime)
        .on_event(move |event, client| {
            let ctx = ctx.clone();
            async move {
                event_bridge::dispatch(ctx, event, client).await;
            }
        });

    if skip_history {
        builder = builder.skip_history_sync();
    }
    if let Some(options) = pair_code {
        builder = builder.with_pair_code(options);
    }

    builder.build().await.map_err(|e| anyhow::anyhow!(e))
}
