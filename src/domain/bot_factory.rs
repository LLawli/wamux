//! Build a `whatsapp_rust::Bot` for one account, wired to the storage backend
//! the engine minted for it, tokio transport/runtime, ureq HTTP, and the wamux
//! event bridge. Engine-agnostic: it only ever sees `dyn Backend`.

use std::sync::Arc;

use whatsapp_rust::TokioRuntime;
use whatsapp_rust::bot::Bot;
use whatsapp_rust::pair_code::PairCodeOptions;
use whatsapp_rust::store::traits::Backend;
use whatsapp_rust_tokio_transport::{Connector, TokioWebSocketTransportFactory};
use whatsapp_rust_ureq_http_client::UreqHttpClient;

use crate::state::event_bridge::{self, EventCtx};

/// Construct (but do not run) a Bot on `backend` (already scoped to one
/// account's device_id). Pass `pair_code` to start phone-number pairing on
/// connect; omit it to pair via QR (emitted as events).
///
/// `skip_history`: relay-pure default. When `true` the phone's history dump is
/// dropped (receipt sent so it stops retrying); when `false` it is processed and
/// emitted as `HistorySyncEvent`s for the edge to backfill (e.g. a CRM).
pub async fn build_bot(
    backend: Arc<dyn Backend>,
    ctx: EventCtx,
    pair_code: Option<PairCodeOptions>,
    skip_history: bool,
    ws_url: Option<&str>,
) -> anyhow::Result<Bot> {
    reject_non_loopback_under_stress(ws_url)?;

    // `ws_url` overrides the upstream endpoint (stress mock); plaintext ws://
    // needs the Plain connector since the default forces TLS.
    let transport = match ws_url {
        Some(url) => TokioWebSocketTransportFactory::new()
            .with_url(url)
            .with_connector(Connector::Plain),
        None => TokioWebSocketTransportFactory::new(),
    };

    let mut builder = Bot::builder()
        // 0.7 split the setter: `with_backend` takes an owned `impl Backend`
        // and boxes it, so an already-shared `Arc<dyn Backend>` (what the
        // engine hands us, one per account) goes through the `_arc` form.
        .with_backend_arc(backend)
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

/// A build carrying the `stress` feature has the Noise server-certificate chain
/// check DISABLED (`wacore-noise/danger-skip-cert-chain-verify`), because the
/// mock signs its chain with zeros and whatsapp-rust 0.7 started verifying the
/// intermediate's XEdDSA signature. Such a build cannot tell the real WhatsApp
/// from anything else holding the socket, so it must never reach it.
///
/// This is the choke point: every connect goes through `build_bot`, so refusing
/// a non-loopback endpoint here makes "stress build talks to production" a
/// startup-shaped failure instead of a silently unauthenticated session.
///
/// The check is deliberately allowlist-shaped (loopback only), not a blocklist
/// of WhatsApp hostnames: a blocklist would pass a proxy, a DNS override, or a
/// hostname WhatsApp adds tomorrow.
#[cfg(feature = "stress")]
fn reject_non_loopback_under_stress(ws_url: Option<&str>) -> anyhow::Result<()> {
    let url = ws_url.ok_or_else(|| {
        anyhow::anyhow!(
            "this binary was built with the `stress` feature, which disables Noise \
             certificate-chain verification; it refuses to connect to the default \
             (real) WhatsApp endpoint. Set the ws_url override to a loopback mock, \
             or rebuild without `--features stress`."
        )
    })?;
    if !is_loopback_ws_url(url) {
        anyhow::bail!(
            "this binary was built with the `stress` feature, which disables Noise \
             certificate-chain verification; it only connects to a loopback mock, \
             and '{url}' is not one. Rebuild without `--features stress` to reach a \
             real WhatsApp endpoint."
        );
    }
    Ok(())
}

/// No-op in a normal build: certificate-chain verification is on, so any
/// endpoint is the operator's call.
#[cfg(not(feature = "stress"))]
fn reject_non_loopback_under_stress(_ws_url: Option<&str>) -> anyhow::Result<()> {
    Ok(())
}

/// Whether a `ws://`/`wss://` URL points at this machine. `localhost` is
/// accepted by name; everything else has to parse as a loopback IP, so a host
/// that merely *resolves* to 127.0.0.1 today is still refused.
#[cfg(feature = "stress")]
fn is_loopback_ws_url(url: &str) -> bool {
    let Ok(uri) = url.parse::<http::Uri>() else {
        return false;
    };
    match uri.host() {
        None => false,
        Some("localhost") => true,
        // http::Uri keeps the brackets on an IPv6 literal host.
        Some(host) => host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback()),
    }
}

#[cfg(all(test, feature = "stress"))]
mod stress_endpoint_guard_tests {
    use super::{is_loopback_ws_url, reject_non_loopback_under_stress};

    #[test]
    fn loopback_endpoints_are_accepted() {
        assert!(is_loopback_ws_url("ws://127.0.0.1:42963"));
        assert!(is_loopback_ws_url("ws://127.0.0.5:1/ws/chat"));
        assert!(is_loopback_ws_url("ws://localhost:8080"));
        assert!(is_loopback_ws_url("ws://[::1]:8080"));
    }

    #[test]
    fn the_real_whatsapp_endpoint_is_refused() {
        assert!(!is_loopback_ws_url("wss://web.whatsapp.com/ws/chat"));
        assert!(reject_non_loopback_under_stress(Some("wss://web.whatsapp.com/ws/chat")).is_err());
    }

    // The default (None) means "the real endpoint", which is exactly what a
    // cert-check-disabled build must not reach.
    #[test]
    fn the_default_endpoint_is_refused() {
        assert!(reject_non_loopback_under_stress(None).is_err());
    }

    // A hostname that resolves to loopback is still refused: the guard must not
    // depend on whatever DNS answers at connect time.
    #[test]
    fn a_non_loopback_host_is_refused_even_if_it_could_resolve_locally() {
        assert!(!is_loopback_ws_url("ws://localtest.me:8080"));
        assert!(!is_loopback_ws_url("ws://10.0.0.1:8080"));
        assert!(!is_loopback_ws_url("not a url at all"));
    }
}
