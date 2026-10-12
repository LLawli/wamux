//! Optional allowlist by `SO_PEERCRED` (#77): with `allowed_uids` or
//! `allowed_gids` set, a connection whose peer is in neither list gets
//! `PermissionDenied` before any RPC runs. Both empty (the default) admits
//! everyone who can open the socket, as before: the socket's mode and group
//! stay the real gate.
//!
//! The gid is the peer's effective gid as the kernel reports it at connect
//! time; its supplementary groups are not visible here.

use std::sync::Arc;
use std::task::{Context, Poll};

use futures::future::{Either, Ready, ready};
use http::{Request, Response};
use tonic::transport::server::UdsConnectInfo;
use tower::{Layer, Service};
use wamux_types::WamuxError;

/// Who may talk to the socket. Empty lists admit everyone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PeerAllowlist {
    uids: Vec<u32>,
    gids: Vec<u32>,
}

impl PeerAllowlist {
    pub fn new(uids: Vec<u32>, gids: Vec<u32>) -> Self {
        Self { uids, gids }
    }

    /// `peer` is the `(uid, gid)` `SO_PEERCRED` gave for the connection, or
    /// `None` when the kernel gave none.
    pub fn admits(&self, peer: Option<(u32, u32)>) -> bool {
        if self.uids.is_empty() && self.gids.is_empty() {
            return true;
        }
        // No credentials with a list set is a denial: nothing proves who it is.
        peer.is_some_and(|(uid, gid)| self.uids.contains(&uid) || self.gids.contains(&gid))
    }
}

/// Tower layer that checks every request against a [`PeerAllowlist`].
#[derive(Clone)]
pub struct PeerAllowlistLayer {
    allowlist: Arc<PeerAllowlist>,
}

impl PeerAllowlistLayer {
    pub fn new(allowlist: PeerAllowlist) -> Self {
        Self {
            allowlist: Arc::new(allowlist),
        }
    }
}

impl<S> Layer<S> for PeerAllowlistLayer {
    type Service = PeerAllowlistService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        PeerAllowlistService {
            inner,
            allowlist: self.allowlist.clone(),
        }
    }
}

#[derive(Clone)]
pub struct PeerAllowlistService<S> {
    inner: S,
    allowlist: Arc<PeerAllowlist>,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for PeerAllowlistService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    ResBody: Default,
{
    type Response = Response<ResBody>;
    type Error = S::Error;
    // Either the inner call, or an already-built refusal: no boxing on the
    // admitted path.
    type Future = Either<S::Future, Ready<Result<Response<ResBody>, S::Error>>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let creds = peer_credentials(&req);
        if self.allowlist.admits(creds) {
            return Either::Left(self.inner.call(req));
        }
        // Built from the typed error so the Status stays in error.rs, the one
        // place that constructs them (scripts/check-status-sites.py).
        let denied = WamuxError::PeerNotAllowed {
            uid: creds.map(|(uid, _)| uid),
            gid: creds.map(|(_, gid)| gid),
        };
        Either::Right(ready(Ok(tonic::Status::from(denied).into_http())))
    }
}

/// `(uid, gid)` from the connection's `SO_PEERCRED`, when the kernel gave it.
fn peer_credentials<ReqBody>(req: &Request<ReqBody>) -> Option<(u32, u32)> {
    let info = req.extensions().get::<UdsConnectInfo>()?;
    let cred = info.peer_cred?;
    Some((cred.uid(), cred.gid()))
}

#[cfg(test)]
#[path = "peer_allowlist_tests.rs"]
mod tests;
