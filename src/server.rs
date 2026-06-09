//! Assemble the tonic server: register all services + (optional) reflection.

use std::sync::Arc;

use tonic::transport::Server;
use tonic::transport::server::Router;
use tower::layer::util::{Identity, Stack};

use crate::config::Config;
use crate::observe::RequestObserveLayer;
use crate::proto::v1::account_service_server::AccountServiceServer;
use crate::proto::v1::admin_service_server::AdminServiceServer;
use crate::proto::v1::contact_service_server::ContactServiceServer;
use crate::proto::v1::event_service_server::EventServiceServer;
use crate::proto::v1::group_service_server::GroupServiceServer;
use crate::proto::v1::media_service_server::MediaServiceServer;
use crate::proto::v1::messaging_service_server::MessagingServiceServer;
use crate::services::account_service::AccountSvc;
use crate::services::admin_service::AdminSvc;
use crate::services::contact_service::ContactSvc;
use crate::services::event_service::EventSvc;
use crate::services::group_service::GroupSvc;
use crate::services::media_service::MediaSvc;
use crate::services::messaging_service::MessagingSvc;
use crate::state::AccountRegistry;

/// The router carries our one cross-cutting layer (per-request observability);
/// the type names that layer so callers can store the router.
pub type ObservedRouter = Router<Stack<RequestObserveLayer, Identity>>;

pub fn build_router(registry: Arc<AccountRegistry>, config: &Config) -> ObservedRouter {
    // Render handle for AdminService.GetMetrics; also installs the global
    // recorder the observability layer below feeds.
    let prometheus = crate::observe::prometheus_handle();
    let mut router = Server::builder()
        .layer(RequestObserveLayer)
        .add_service(AccountServiceServer::new(AccountSvc::new(registry.clone())))
        .add_service(EventServiceServer::new(EventSvc::new(registry.clone())))
        .add_service(MessagingServiceServer::new(MessagingSvc::new(
            registry.clone(),
            config.media_max_bytes,
        )))
        .add_service(MediaServiceServer::new(MediaSvc::new(registry.clone())))
        .add_service(GroupServiceServer::new(GroupSvc::new(registry.clone())))
        .add_service(ContactServiceServer::new(ContactSvc::new(registry.clone())))
        .add_service(AdminServiceServer::new(AdminSvc::new(
            registry.clone(),
            prometheus,
        )));

    if config.enable_reflection {
        // Startup invariant: the descriptor is generated from our own protos.
        let reflection = tonic_reflection::server::Builder::configure()
            .register_encoded_file_descriptor_set(crate::proto::FILE_DESCRIPTOR_SET)
            .build_v1()
            .expect("valid embedded file descriptor set");
        router = router.add_service(reflection);
    }

    router
}
