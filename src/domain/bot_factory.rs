//! Build a `whatsapp_rust::Bot` for one account, wired to the storage backend
//! the engine minted for it, tokio transport/runtime, ureq HTTP, and the wamux
//! event bridge. Engine-agnostic: it only ever sees `dyn Backend`.

use std::sync::Arc;

use wacore::handshake::NoiseCertPolicy;
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
    // `noise_cert_policy` folds the loopback guard in: a stress build only
    // gets `DangerSkipCertChainVerify` after it, so this call is the single
    // choke point for both "which policy" and "is this endpoint allowed".
    let cert_policy = noise_cert_policy(ws_url)?;

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
        .with_noise_cert_policy(cert_policy)
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

/// A build carrying the `stress` feature can skip the Noise server-certificate
/// chain check (`noise_cert_policy`), because the mock signs its chain with
/// zeros and whatsapp-rust verifies the intermediate's XEdDSA signature. Such a
/// client cannot tell the real WhatsApp from anything else holding the socket,
/// so a stress build must never reach it.
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

/// The Noise certificate policy for a client that will connect to `ws_url`.
///
/// Since upstream #1441/#1444 (#30) skipping the chain check is a per-client
/// runtime policy, not a cargo feature that disabled it for the whole build.
/// This is the one place wamux chooses it, welded to the loopback guard:
///
/// - a normal build always answers `NoiseCertPolicy::Strict`, whatever the URL;
/// - a `stress` build answers `DangerSkipCertChainVerify` for a loopback mock,
///   and refuses anything else exactly as `reject_non_loopback_under_stress`.
///
/// `build_bot` must take the policy from here and hand it to the builder.
#[cfg(feature = "stress")]
pub(crate) fn noise_cert_policy(ws_url: Option<&str>) -> anyhow::Result<NoiseCertPolicy> {
    reject_non_loopback_under_stress(ws_url)?;
    Ok(NoiseCertPolicy::DangerSkipCertChainVerify)
}

#[cfg(not(feature = "stress"))]
pub(crate) fn noise_cert_policy(_ws_url: Option<&str>) -> anyhow::Result<NoiseCertPolicy> {
    Ok(NoiseCertPolicy::Strict)
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

#[cfg(all(test, feature = "stress"))]
mod stress_cert_policy_tests {
    use super::{NoiseCertPolicy, noise_cert_policy};

    // The mock signs its chain with zeros, so without this the stress suite
    // cannot finish a handshake at all.
    #[test]
    fn a_loopback_mock_gets_the_danger_policy() {
        assert_eq!(
            noise_cert_policy(Some("ws://127.0.0.1:42963")).unwrap(),
            NoiseCertPolicy::DangerSkipCertChainVerify
        );
    }

    // The policy may only be chosen after the guard passes.
    #[test]
    fn a_stress_build_still_refuses_the_real_endpoint() {
        assert!(noise_cert_policy(None).is_err());
        assert!(noise_cert_policy(Some("wss://web.whatsapp.com/ws/chat")).is_err());
    }
}

#[cfg(all(test, not(feature = "stress")))]
mod cert_policy_tests {
    use super::{NoiseCertPolicy, noise_cert_policy};

    // Production never skips the chain check, not even for a loopback URL:
    // the danger policy exists only in a `stress` build.
    #[test]
    fn a_normal_build_always_verifies_the_chain() {
        for url in [
            None,
            Some("ws://127.0.0.1:42963"),
            Some("wss://web.whatsapp.com/ws/chat"),
        ] {
            assert_eq!(noise_cert_policy(url).unwrap(), NoiseCertPolicy::Strict);
        }
    }
}
