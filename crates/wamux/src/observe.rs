//! Observability: the process-global Prometheus recorder and the per-request
//! tower layer that emits one log line + metrics per gRPC call.
//!
//! The layer wraps the response body so it can read the `grpc-status` trailer at
//! end-of-stream (unary status lives in trailers, or in headers for a
//! trailers-only error response). Peer uid comes from `SO_PEERCRED`, which tonic
//! puts in the request extensions as `UdsConnectInfo` before the layer runs.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};
use std::time::Instant;

use http::{Extensions, HeaderMap, Request, Response};
use http_body::{Body, Frame, SizeHint};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use prost::Message;
use tonic::transport::server::UdsConnectInfo;
use tower::{Layer, Service};

/// The process-global Prometheus recorder, installed once. The `metrics` facade
/// is global by design (not deployment-varying config), so a single install
/// behind `OnceLock` is the idiomatic exception to the "no globals" rule.
static PROMETHEUS: OnceLock<PrometheusHandle> = OnceLock::new();

/// Install (idempotently) the Prometheus recorder and return a render handle.
/// Must be called from within a tokio runtime (the recorder spawns an upkeep
/// task); `build_router` does this while `main`/tests are already async.
pub fn prometheus_handle() -> PrometheusHandle {
    PROMETHEUS
        .get_or_init(|| {
            // Startup invariant: the only way this fails is a double global
            // install, which the OnceLock already prevents.
            PrometheusBuilder::new()
                .install_recorder()
                .expect("install prometheus recorder")
        })
        .clone()
}

/// The gRPC methods this daemon serves, as `pkg.Service/Method` (#80). A
/// metric is labelled with the method only when the path is one of these, and
/// `unknown` otherwise, so a client calling made-up paths cannot create a new
/// Prometheus series per path.
#[derive(Debug, Clone)]
pub struct KnownMethods {
    methods: HashSet<Arc<str>>,
    // Shared so an unknown request clones a pointer instead of allocating.
    unknown: Arc<str>,
}

/// The reflection service is added by `tonic-reflection`, not by our protos,
/// so the descriptor set does not list it.
const REFLECTION_METHOD: &str = "grpc.reflection.v1.ServerReflection/ServerReflectionInfo";

/// The `method` label for any path that is not a served method.
const UNKNOWN_LABEL: &str = "unknown";

/// Longest raw path the log line carries for an unknown method (#80): the
/// client controls the path, so the log must not grow with it either.
const LOGGED_PATH_MAX_CHARS: usize = 128;

impl KnownMethods {
    /// Every method of every service in `bytes` (an encoded
    /// `FileDescriptorSet`), plus the reflection service's.
    pub fn from_descriptor_set(bytes: &[u8]) -> Result<Self, prost::DecodeError> {
        let set = prost_types::FileDescriptorSet::decode(bytes)?;
        let mut methods: HashSet<Arc<str>> = HashSet::new();
        methods.insert(Arc::from(REFLECTION_METHOD));
        for file in &set.file {
            for service in &file.service {
                for method in &service.method {
                    let path = format!("{}.{}/{}", file.package(), service.name(), method.name());
                    methods.insert(Arc::from(path));
                }
            }
        }
        Ok(Self {
            methods,
            unknown: Arc::from(UNKNOWN_LABEL),
        })
    }

    /// The methods of `wamux.v1` and reflection.
    pub fn wamux() -> Self {
        // Startup invariant: the descriptor set is generated from our own
        // protos at build time, so it always decodes (same stance as
        // `prometheus_handle`).
        Self::from_descriptor_set(crate::proto::FILE_DESCRIPTOR_SET)
            .expect("decode our own descriptor set")
    }

    /// The label for a request path: the method, or `unknown`.
    pub fn label<'a>(&'a self, path: &str) -> &'a str {
        self.label_shared(path)
    }

    /// Same as `label`, as a cheap-to-clone handle for the response body.
    fn label_shared(&self, path: &str) -> &Arc<str> {
        let method = path.strip_prefix('/').unwrap_or(path);
        self.methods.get(method).unwrap_or(&self.unknown)
    }
}

/// Tower layer that times each request and logs/records its outcome.
#[derive(Clone)]
pub struct RequestObserveLayer {
    known: Arc<KnownMethods>,
}

impl RequestObserveLayer {
    pub fn new(known: KnownMethods) -> Self {
        Self {
            known: Arc::new(known),
        }
    }
}

impl<S> Layer<S> for RequestObserveLayer {
    type Service = RequestObserve<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequestObserve {
            inner,
            known: self.known.clone(),
        }
    }
}

#[derive(Clone)]
pub struct RequestObserve<S> {
    inner: S,
    known: Arc<KnownMethods>,
}

impl<S> RequestObserve<S> {
    /// The metric label and, for an unknown method only, the raw path
    /// truncated for the log (#80).
    fn label_request(&self, path: &str) -> (Arc<str>, Option<String>) {
        let method = self.known.label_shared(path).clone();
        let raw_path =
            (*method == *UNKNOWN_LABEL).then(|| path.chars().take(LOGGED_PATH_MAX_CHARS).collect());
        (method, raw_path)
    }
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for RequestObserve<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    S::Future: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = Response<ObservedBody<ResBody>>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let (method, raw_path) = self.label_request(req.uri().path());
        let peer_uid = peer_uid(req.extensions());
        let start = Instant::now();
        let fut = self.inner.call(req);
        Box::pin(async move {
            let response = fut.await?;
            let (parts, body) = response.into_parts();
            // Trailers-only error responses carry grpc-status in the headers.
            let status = status_code(&parts.headers);
            let body = ObservedBody {
                inner: body,
                start,
                method,
                raw_path,
                peer_uid,
                status,
            };
            Ok(Response::from_parts(parts, body))
        })
    }
}

/// Wraps the response body: captures the final `grpc-status` and emits the log
/// line + metrics exactly once, on drop (i.e. when the response is fully sent or
/// the client goes away). Drop is the single emit point so streaming responses
/// are timed over their whole lifetime.
pub struct ObservedBody<B> {
    inner: B,
    start: Instant,
    method: Arc<str>,
    /// Raw path, kept only for an unknown method, so the log can show it.
    raw_path: Option<String>,
    peer_uid: Option<u32>,
    status: Option<String>,
}

impl<B> Body for ObservedBody<B>
where
    B: Body + Unpin,
{
    type Data = B::Data;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        // `Self: Unpin` (all fields are), so a plain `get_mut` is sound.
        let this = self.get_mut();
        let frame = std::task::ready!(Pin::new(&mut this.inner).poll_frame(cx));
        if let Some(Ok(f)) = frame.as_ref()
            && let Some(trailers) = f.trailers_ref()
        {
            this.status = grpc_status(trailers);
        }
        Poll::Ready(frame)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

impl<B> Drop for ObservedBody<B> {
    fn drop(&mut self) {
        let latency = self.start.elapsed();
        let status = code_name(self.status.as_deref());
        metrics::counter!(
            "wamux_grpc_requests_total",
            "method" => self.method.to_string(),
            "status" => status,
        )
        .increment(1);
        metrics::histogram!(
            "wamux_grpc_request_duration_seconds",
            "method" => self.method.to_string(),
        )
        .record(latency.as_secs_f64());
        tracing::info!(
            method = %self.method,
            path = self.raw_path.as_deref(),
            peer_uid = ?self.peer_uid,
            grpc_status = status,
            latency_ms = latency.as_millis() as u64,
            "grpc request"
        );
    }
}

fn peer_uid(extensions: &Extensions) -> Option<u32> {
    extensions
        .get::<UdsConnectInfo>()
        .and_then(|info| info.peer_cred)
        .map(|cred| cred.uid())
}

fn status_code(headers: &HeaderMap) -> Option<String> {
    grpc_status(headers)
}

fn grpc_status(headers: &HeaderMap) -> Option<String> {
    headers
        .get("grpc-status")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// Canonical gRPC status name for a `grpc-status` code (label/log friendly).
/// `None` (absent trailer with no error) means a successful unary call → OK.
fn code_name(code: Option<&str>) -> &'static str {
    match code {
        None | Some("0") => "ok",
        Some("1") => "cancelled",
        Some("2") => "unknown",
        Some("3") => "invalid_argument",
        Some("4") => "deadline_exceeded",
        Some("5") => "not_found",
        Some("6") => "already_exists",
        Some("7") => "permission_denied",
        Some("8") => "resource_exhausted",
        Some("9") => "failed_precondition",
        Some("10") => "aborted",
        Some("11") => "out_of_range",
        Some("12") => "unimplemented",
        Some("13") => "internal",
        Some("14") => "unavailable",
        Some("15") => "data_loss",
        Some("16") => "unauthenticated",
        Some(_) => "unknown",
    }
}

#[cfg(test)]
#[path = "observe_tests.rs"]
mod tests;
