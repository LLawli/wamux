//! Server reflection over the real Unix socket (#61). `grpcurl -unix list` is
//! how a human pokes the daemon, and it only works while reflection serves the
//! descriptor the codegen emits. A codegen swap (tonic-build -> tonic-prost-build)
//! can silently change what lands in that descriptor, so both directions are
//! pinned: every service of the contract is listed, and the on-disk blob format
//! (#31), compiled by the same build script, stays out of it.
//!
//! Runs on SQLite, so it needs no container.

use std::sync::Arc;
use std::time::Duration;

use tokio_stream::StreamExt;
use tonic::transport::Channel;
use tonic_reflection::pb::v1::server_reflection_client::ServerReflectionClient;
use tonic_reflection::pb::v1::server_reflection_request::MessageRequest;
use tonic_reflection::pb::v1::server_reflection_response::MessageResponse;
use tonic_reflection::pb::v1::{ServerReflectionRequest, ServerReflectionResponse};

use wamux::config::Config;
use wamux::state::{AccountRegistry, RegistryTuning};
use wamux::transport::shutdown::Shutdown;
use wamux::{server, transport};

#[allow(dead_code)]
mod common;

/// Every service in `proto/*.proto`, plus reflection itself.
const EXPECTED_SERVICES: [&str; 9] = [
    "grpc.reflection.v1.ServerReflection",
    "wamux.v1.AccountService",
    "wamux.v1.AdminService",
    "wamux.v1.ContactService",
    "wamux.v1.EventService",
    "wamux.v1.GroupService",
    "wamux.v1.MediaService",
    "wamux.v1.MessagingService",
    "wamux.v1.NewsletterService",
];

/// A daemon with reflection on, served on a throwaway socket. The temp dir is
/// returned so the socket outlives the test body.
async fn reflection_client() -> (ServerReflectionClient<Channel>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("wamux.sock");
    let socket_str = socket.to_str().unwrap().to_string();
    let (engine, db) = common::sqlite_engine().await;
    Box::leak(Box::new(db));
    let registry = Arc::new(AccountRegistry::new(engine, RegistryTuning::with_ring(8)));
    let config = Config {
        socket_path: socket_str.clone(),
        enable_reflection: true,
        ..Config::default()
    };
    let incoming = transport::uds_listener::bind(&socket_str, 0o660, None).expect("bind");
    let shutdown = Shutdown::new();
    let router = server::build_router(registry, &config, shutdown.clone());
    tokio::spawn(server::serve_until_shutdown(
        router,
        incoming,
        shutdown,
        Duration::from_secs(5),
    ));
    let client = ServerReflectionClient::new(common::uds_channel(&socket).await);
    (client, dir)
}

/// One request on the bidirectional reflection stream, one answer back. An
/// unknown symbol comes back as a `NotFound` status on the stream, not as an
/// `ErrorResponse` message, so the stream's status is part of the answer. Only
/// its code is kept: the tests need no more, and a whole `Status` is too large
/// an `Err` for clippy.
async fn ask(
    client: &mut ServerReflectionClient<Channel>,
    request: MessageRequest,
) -> Result<MessageResponse, tonic::Code> {
    let outbound = tokio_stream::once(ServerReflectionRequest {
        host: String::new(),
        message_request: Some(request),
    });
    let mut inbound = client
        .server_reflection_info(outbound)
        .await
        .expect("open reflection stream")
        .into_inner();
    let answer: ServerReflectionResponse = inbound
        .next()
        .await
        .expect("reflection answered nothing")
        .map_err(|status| status.code())?;
    Ok(answer
        .message_response
        .expect("reflection answer carried no response"))
}

#[tokio::test]
async fn reflection_lists_every_wamux_service() {
    let (mut client, _dir) = reflection_client().await;
    let answer = ask(&mut client, MessageRequest::ListServices(String::new()))
        .await
        .expect("list services");
    let MessageResponse::ListServicesResponse(list) = answer else {
        panic!("expected a service list, got {answer:?}");
    };
    let mut listed: Vec<String> = list.service.into_iter().map(|s| s.name).collect();
    listed.sort();
    assert_eq!(listed, EXPECTED_SERVICES.map(String::from).to_vec());
}

#[tokio::test]
async fn reflection_does_not_expose_the_store_blob_format() {
    let (mut client, _dir) = reflection_client().await;
    // Control: a contract symbol resolves, so an error below means "not in the
    // descriptor", not "reflection is broken".
    let contract = ask(
        &mut client,
        MessageRequest::FileContainingSymbol("wamux.v1.AccountService".into()),
    )
    .await;
    assert!(
        matches!(contract, Ok(MessageResponse::FileDescriptorResponse(_))),
        "a contract symbol must resolve, got {contract:?}"
    );
    let blob = ask(
        &mut client,
        MessageRequest::FileContainingSymbol("wamux.store.HashStateWire".into()),
    )
    .await;
    let absent = match &blob {
        Err(code) => *code == tonic::Code::NotFound,
        Ok(response) => matches!(response, MessageResponse::ErrorResponse(_)),
    };
    assert!(
        absent,
        "the on-disk blob format leaked into the socket's descriptor: {blob:?}"
    );
}
