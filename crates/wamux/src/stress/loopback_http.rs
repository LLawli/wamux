//! The production HTTP client, pointed at a loopback CDN (#69).
//!
//! whatsapp-rust always builds a media URL as `https://{host}{direct_path}`
//! (`wacore/src/download.rs`), and the production client (`UreqHttpClient`)
//! trusts only public roots, so a CDN standing in on 127.0.0.1 cannot answer
//! it. This wraps that same client and rewrites `https://` to `http://` for a
//! loopback host only: a real CDN host is never loopback, so nothing a
//! production server names can be downgraded. Compiled only with the `stress`
//! feature, like the mock it serves.

use anyhow::Result;
use async_trait::async_trait;
use wacore::net::{HttpClient, HttpRequest, HttpResponse, StreamingHttpResponse, UploadBody};
use whatsapp_rust_ureq_http_client::UreqHttpClient;

const LOOPBACK_HOSTS: [&str; 3] = ["127.0.0.1", "localhost", "[::1]"];

/// `Some(url with http://)` when `url` is `https://` to a loopback host
/// (`127.0.0.1`, `localhost` or `[::1]`, with or without a port); `None` for
/// anything else, including a host that merely starts with a loopback name.
pub fn loopback_to_plain_http(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // Userinfo can disguise the real host (`user@evil`), so refuse it outright.
    if authority.contains('@') {
        return None;
    }
    LOOPBACK_HOSTS
        .contains(&strip_port(authority))
        .then(|| format!("http://{rest}"))
}

/// The host part of `host[:port]`, keeping the brackets of an IPv6 literal.
fn strip_port(authority: &str) -> &str {
    if authority.starts_with('[') {
        return authority.split_inclusive(']').next().unwrap_or_default();
    }
    authority.split(':').next().unwrap_or_default()
}

/// `UreqHttpClient`, with loopback `https://` URLs sent as `http://`.
pub struct LoopbackHttpClient {
    inner: UreqHttpClient,
}

impl LoopbackHttpClient {
    pub fn new() -> Self {
        Self {
            inner: UreqHttpClient::new(),
        }
    }
}

impl Default for LoopbackHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Apply the loopback rewrite to a request's URL, leaving the rest intact.
fn rewritten(mut request: HttpRequest) -> HttpRequest {
    if let Some(plain) = loopback_to_plain_http(&request.url) {
        request.url = plain;
    }
    request
}

#[async_trait]
impl HttpClient for LoopbackHttpClient {
    async fn execute(&self, request: HttpRequest) -> Result<HttpResponse> {
        self.inner.execute(rewritten(request)).await
    }

    fn supports_streaming(&self) -> bool {
        self.inner.supports_streaming()
    }

    fn execute_streaming(&self, request: HttpRequest) -> Result<StreamingHttpResponse> {
        self.inner.execute_streaming(rewritten(request))
    }

    fn supports_upload_streaming(&self) -> bool {
        self.inner.supports_upload_streaming()
    }

    fn execute_upload(
        &self,
        request: HttpRequest,
        body: UploadBody,
        content_length: u64,
    ) -> Result<HttpResponse> {
        self.inner
            .execute_upload(rewritten(request), body, content_length)
    }

    fn resource_report(&self) -> Option<wacore::stats::HttpResourceReport> {
        self.inner.resource_report()
    }
}

#[cfg(test)]
mod tests;
