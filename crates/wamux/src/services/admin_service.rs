//! AdminService: the daemon's observability surface, exposed over the socket
//! (no TCP listener). `GetMetrics` renders the Prometheus registry fed by the
//! per-request layer plus live account gauges; `Check` reports liveness,
//! readiness (storage reachable) and the store's `blob_format` marker, read live
//! (#84). Health here is about the daemon itself, not per-account WhatsApp state
//! (that rides the event stream).

use std::sync::Arc;

use metrics_exporter_prometheus::PrometheusHandle;
use tonic::{Request, Response, Status};

use crate::proto::v1 as pb;
use crate::proto::v1::admin_service_server::AdminService;
use crate::state::AccountRegistry;
use crate::storage::{StorageEngine, StoreBlobFormat};

pub struct AdminSvc {
    registry: Arc<AccountRegistry>,
    prometheus: PrometheusHandle,
}

impl AdminSvc {
    pub fn new(registry: Arc<AccountRegistry>, prometheus: PrometheusHandle) -> Self {
        Self {
            registry,
            prometheus,
        }
    }

    /// Publish the live account gauges right before a render (they reflect the
    /// registry's current state, not an accumulating counter).
    fn publish_account_gauges(&self) {
        let accounts = self.registry.list();
        let total = accounts.len();
        let connected = accounts
            .iter()
            .filter(|h| h.current_state() == pb::ConnectionState::Connected)
            .count();
        metrics::gauge!("wamux_accounts_total").set(total as f64);
        metrics::gauge!("wamux_accounts_connected").set(connected as f64);
    }
}

#[tonic::async_trait]
impl AdminService for AdminSvc {
    async fn get_metrics(
        &self,
        _request: Request<pb::Empty>,
    ) -> Result<Response<pb::MetricsResponse>, Status> {
        self.publish_account_gauges();
        let prometheus = self.prometheus.render();
        Ok(Response::new(pb::MetricsResponse { prometheus }))
    }

    async fn check(
        &self,
        _request: Request<pb::Empty>,
    ) -> Result<Response<pb::HealthResponse>, Status> {
        // Readiness probes storage with a trivial round-trip; failure is not an
        // RPC error (the caller wants `ready=false`, not a Status).
        let ready = self.registry.storage().ping_storage().await;
        let blob_format = reported_blob_format(self.registry.storage().as_ref()).await;
        Ok(Response::new(pb::HealthResponse {
            serving: true,
            ready,
            version: env!("CARGO_PKG_VERSION").to_string(),
            blob_format: blob_format as i32,
        }))
    }
}

/// The store's `blob_format` on the wire (#84). A store that does not answer,
/// or a marker this build does not know, is `Unspecified`: health never fails.
async fn reported_blob_format(storage: &dyn StorageEngine) -> pb::BlobFormat {
    match storage.blob_format().await {
        Ok(StoreBlobFormat::Bincode) => pb::BlobFormat::Bincode,
        Ok(StoreBlobFormat::Protobuf) => pb::BlobFormat::Protobuf,
        Err(error) => {
            tracing::warn!(%error, "health check could not read the blob_format marker (#84)");
            pb::BlobFormat::Unspecified
        }
    }
}
