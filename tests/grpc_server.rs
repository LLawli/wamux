//! M3 gate: exercise the account-lifecycle RPCs over a real Unix-socket gRPC
//! connection (the same path production uses). Runs against whichever engine
//! WAMUX_TEST_ENGINE names (default Postgres, which needs the docker container).

use std::sync::Arc;
use std::time::Duration;

use hyper_util::rt::TokioIo;
use tonic::transport::{Endpoint, Uri};
use tower::service_fn;

use wacore::store::traits::LidPnMappingEntry;
use wamux::config::Config;
use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::admin_service_client::AdminServiceClient;
use wamux::proto::v1::contact_service_client::ContactServiceClient;
use wamux::state::{AccountRegistry, RegistryTuning};
use wamux::storage::StorageEngine;
use wamux::{server, transport};

// Only a subset of the shared helpers is used per test binary.
#[allow(dead_code)]
mod common;

fn account_ref(uuid: &str) -> pb::AccountRef {
    pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::Uuid(uuid.to_string())),
    }
}

#[tokio::test]
async fn account_lifecycle_over_socket() {
    // --- server ---
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("wamux.sock");
    let socket_str = socket.to_str().unwrap().to_string();

    let engine = common::test_engine().await;
    let registry = Arc::new(AccountRegistry::new(engine, RegistryTuning::with_ring(64)));
    let config = Config {
        socket_path: socket_str.clone(),
        enable_reflection: false,
        ..Config::default()
    };
    let stream = transport::uds_listener::bind(&socket_str, 0o660, None).expect("bind");
    let router = server::build_router(registry, &config);
    tokio::spawn(async move {
        let _ = router.serve_with_incoming(stream).await;
    });

    // --- client over the UDS ---
    let connect_path = socket.clone();
    let channel = {
        let mut attempt = Endpoint::try_from("http://[::1]:50051")
            .unwrap()
            .connect_with_connector(service_fn(move |_: Uri| {
                let path = connect_path.clone();
                async move {
                    let stream = tokio::net::UnixStream::connect(path).await?;
                    Ok::<_, std::io::Error>(TokioIo::new(stream))
                }
            }))
            .await;
        // small retry while the server task spins up
        for _ in 0..10 {
            if attempt.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            let path = socket.clone();
            attempt = Endpoint::try_from("http://[::1]:50051")
                .unwrap()
                .connect_with_connector(service_fn(move |_: Uri| {
                    let path = path.clone();
                    async move {
                        let stream = tokio::net::UnixStream::connect(path).await?;
                        Ok::<_, std::io::Error>(TokioIo::new(stream))
                    }
                }))
                .await;
        }
        attempt.expect("connect over uds")
    };
    let mut client = AccountServiceClient::new(channel);

    // create
    let external = format!("grpc-test-{}", uuid::Uuid::new_v4());
    let created = client
        .create_account(pb::CreateAccountRequest {
            external_ref: Some(external.clone()),
        })
        .await
        .expect("create_account")
        .into_inner();
    assert!(!created.uuid.is_empty());
    assert_eq!(created.external_ref, external);
    assert_eq!(created.state, pb::ConnectionState::Disconnected as i32);

    // list contains it
    let listed = client
        .list_accounts(pb::ListAccountsRequest {})
        .await
        .expect("list")
        .into_inner();
    assert!(listed.accounts.iter().any(|a| a.uuid == created.uuid));

    // status by uuid
    let status = client
        .get_account_status(account_ref(&created.uuid))
        .await
        .expect("status")
        .into_inner();
    assert_eq!(status.uuid, created.uuid);

    // resolve by external_ref too
    let by_ext = client
        .get_account_status(pb::AccountRef {
            r#ref: Some(pb::account_ref::Ref::ExternalRef(external.clone())),
        })
        .await
        .expect("status by external_ref")
        .into_inner();
    assert_eq!(by_ext.uuid, created.uuid);

    // unknown account => NotFound
    let missing = client
        .get_account_status(account_ref(&uuid::Uuid::new_v4().to_string()))
        .await;
    assert_eq!(missing.unwrap_err().code(), tonic::Code::NotFound);

    // logout on a never-connected account => FailedPrecondition (real unlink
    // needs a live connection; the edge decides whether to connect first).
    let logout = client.logout(account_ref(&created.uuid)).await;
    assert_eq!(logout.unwrap_err().code(), tonic::Code::FailedPrecondition);

    // delete
    client
        .delete_account(account_ref(&created.uuid))
        .await
        .expect("delete");
    let after = client.get_account_status(account_ref(&created.uuid)).await;
    assert_eq!(after.unwrap_err().code(), tonic::Code::NotFound);
}

/// Spin the server on a throwaway socket and return a connected channel plus
/// the engine behind it (the LID-mapping suite writes through the same storage
/// the RPC reads).
async fn spawn_server() -> (tonic::transport::Channel, Arc<dyn StorageEngine>) {
    let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
    let socket = dir.path().join("wamux.sock");
    let socket_str = socket.to_str().unwrap().to_string();

    let engine = common::test_engine().await;
    let registry = Arc::new(AccountRegistry::new(
        engine.clone(),
        RegistryTuning::with_ring(64),
    ));
    let config = Config {
        socket_path: socket_str.clone(),
        enable_reflection: false,
        ..Config::default()
    };
    let stream = transport::uds_listener::bind(&socket_str, 0o660, None).expect("bind");
    let router = server::build_router(registry, &config);
    tokio::spawn(async move {
        let _ = router.serve_with_incoming(stream).await;
    });

    for _ in 0..10 {
        let path = socket.clone();
        let attempt = Endpoint::try_from("http://[::1]:50051")
            .unwrap()
            .connect_with_connector(service_fn(move |_: Uri| {
                let path = path.clone();
                async move {
                    let stream = tokio::net::UnixStream::connect(path).await?;
                    Ok::<_, std::io::Error>(TokioIo::new(stream))
                }
            }))
            .await;
        if let Ok(channel) = attempt {
            return (channel, engine);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("server never came up");
}

#[tokio::test]
async fn admin_health_and_metrics_over_socket() {
    let (channel, _engine) = spawn_server().await;
    let mut admin = AdminServiceClient::new(channel);

    // Health: serving always true while answering; ready true since PG is up.
    let health = admin.check(pb::Empty {}).await.expect("check").into_inner();
    assert!(health.serving);
    assert!(health.ready);
    assert_eq!(health.version, env!("CARGO_PKG_VERSION"));

    // Metrics: real Prometheus render. Gauges are set synchronously in the
    // handler; the per-request counter is fed by the observability layer when a
    // response body drops, which can lag the client slightly, so poll a few
    // times for it (each render is itself another counted request).
    let mut last = String::new();
    for _ in 0..20 {
        last = admin
            .get_metrics(pb::Empty {})
            .await
            .expect("get_metrics")
            .into_inner()
            .prometheus;
        assert!(last.contains("wamux_accounts_total"));
        if last.contains("wamux_grpc_requests_total") && last.contains("AdminService") {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("per-request metrics never appeared:\n{last}");
}

/// issue #1: the LID<->PN pairs the library persists have to be reachable over
/// the contract, or a `@lid` chat is nameless. `ListLidMappings` is the
/// storage-side read, so it answers for an account that was never connected;
/// the client-side reads (`ResolveLidPn`, `GetPushName`) need a live client and
/// say so instead of lying with an empty answer.
#[tokio::test]
async fn lid_mappings_are_readable_over_socket() {
    let (channel, engine) = spawn_server().await;
    let mut accounts = AccountServiceClient::new(channel.clone());
    let mut contacts = ContactServiceClient::new(channel);

    let external = format!("lid-map-test-{}", uuid::Uuid::new_v4());
    let created = accounts
        .create_account(pb::CreateAccountRequest {
            external_ref: Some(external.clone()),
        })
        .await
        .expect("create_account")
        .into_inner();

    // Seed one pair through the same store the library writes to.
    let device_id = engine
        .list_accounts()
        .await
        .expect("list accounts")
        .into_iter()
        .find(|row| row.uuid.to_string() == created.uuid)
        .expect("created account row")
        .device_id;
    engine
        .device_backend(device_id)
        .put_lid_mapping(&LidPnMappingEntry {
            lid: "169815004184633".to_string(),
            phone_number: "5511999000111".to_string(),
            created_at: 1_717_932_000,
            updated_at: 1_717_932_000,
            learning_source: "usync".to_string(),
        })
        .await
        .expect("seed lid mapping");

    let mappings = contacts
        .list_lid_mappings(account_ref(&created.uuid))
        .await
        .expect("list_lid_mappings on a disconnected account")
        .into_inner()
        .mappings;
    let seeded = mappings
        .iter()
        .find(|m| m.lid == "169815004184633@lid")
        .expect("seeded pair must come back");
    assert_eq!(seeded.pn, "5511999000111@s.whatsapp.net");
    assert_eq!(seeded.learning_source, "usync");
    assert_eq!(seeded.created_at, 1_717_932_000);

    // The client-side reads need a connection.
    let resolve = contacts
        .resolve_lid_pn(pb::ResolveLidPnRequest {
            account: Some(account_ref(&created.uuid)),
            jids: vec!["169815004184633@lid".to_string()],
        })
        .await;
    assert_eq!(
        resolve.unwrap_err().code(),
        tonic::Code::FailedPrecondition,
        "ResolveLidPn reads the live client"
    );

    accounts
        .delete_account(account_ref(&created.uuid))
        .await
        .expect("delete");
}
