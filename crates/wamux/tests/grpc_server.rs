//! M3 gate: exercise the account-lifecycle RPCs over a real Unix-socket gRPC
//! connection (the same path production uses). Runs against whichever engine
//! WAMUX_TEST_ENGINE names (default Postgres, which needs the docker container).

use std::time::Duration;

use wacore::store::traits::LidPnMappingEntry;
use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::admin_service_client::AdminServiceClient;
use wamux::proto::v1::contact_service_client::ContactServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;

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
    let (channel, engine) = common::spawn_server().await;
    let prefix = common::test_prefix("grpc_server", "account_lifecycle_over_socket");
    common::sweep_orphans(&engine, &prefix).await;
    let mut client = AccountServiceClient::new(channel);

    // create
    let external = format!("{prefix}{}", uuid::Uuid::new_v4());
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

#[tokio::test]
async fn admin_health_and_metrics_over_socket() {
    let (channel, _engine) = common::spawn_server().await;
    let mut admin = AdminServiceClient::new(channel);

    // Health: serving always true while answering; ready true since PG is up.
    let health = admin.check(pb::Empty {}).await.expect("check").into_inner();
    assert!(health.serving);
    assert!(health.ready);
    assert_eq!(health.version, env!("CARGO_PKG_VERSION"));

    // Metrics: real Prometheus render. Gauges are set synchronously in the
    // handler; the per-request counter is fed by the observability layer when a
    // response body drops, which can lag the client slightly, so poll for it
    // (each render is itself another counted request).
    let prometheus = common::poll_until(
        "per-request metrics to appear",
        Duration::from_secs(5),
        || {
            let mut admin = admin.clone();
            async move {
                let text = admin
                    .get_metrics(pb::Empty {})
                    .await
                    .expect("get_metrics")
                    .into_inner()
                    .prometheus;
                assert!(text.contains("wamux_accounts_total"));
                (text.contains("wamux_grpc_requests_total") && text.contains("AdminService"))
                    .then_some(text)
            }
        },
    )
    .await;
    assert!(prometheus.contains("AdminService"));
}

/// issue #1: the LID<->PN pairs the library persists have to be reachable over
/// the contract, or a `@lid` chat is nameless. `ListLidMappings` is the
/// storage-side read, so it answers for an account that was never connected;
/// the client-side reads (`ResolveLidPn`, `GetPushName`) need a live client and
/// say so instead of lying with an empty answer.
#[tokio::test]
async fn lid_mappings_are_readable_over_socket() {
    let (channel, engine) = common::spawn_server().await;
    let mut accounts = AccountServiceClient::new(channel.clone());
    let mut contacts = ContactServiceClient::new(channel);

    let prefix = common::test_prefix("grpc_server", "lid_mappings_are_readable");
    common::sweep_orphans(&engine, &prefix).await;
    let external = format!("{prefix}{}", uuid::Uuid::new_v4());
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

    // GetPushName takes an AccountRef now: it answers the account's own name.
    let push_name = contacts.get_push_name(account_ref(&created.uuid)).await;
    assert_eq!(
        push_name.unwrap_err().code(),
        tonic::Code::FailedPrecondition
    );

    accounts
        .delete_account(account_ref(&created.uuid))
        .await
        .expect("delete");
}

/// Issue #41: a status is revoked through RevokeStatus, never DeleteMessage.
/// Both answers below are about the request's shape, so they come back as
/// `InvalidArgument` before the account is looked up: a disconnected account
/// is enough, and the edge hears what to fix rather than "not connected".
#[tokio::test]
async fn status_revoke_is_refused_on_the_wrong_shape_over_socket() {
    let (channel, engine) = common::spawn_server().await;
    let prefix = common::test_prefix("grpc_server", "status_revoke_is_refused");
    common::sweep_orphans(&engine, &prefix).await;
    let mut accounts = AccountServiceClient::new(channel.clone());
    let mut messaging = MessagingServiceClient::new(channel);
    let created = accounts
        .create_account(pb::CreateAccountRequest {
            external_ref: Some(format!("{prefix}{}", uuid::Uuid::new_v4())),
        })
        .await
        .expect("create_account")
        .into_inner();
    let account = Some(account_ref(&created.uuid));

    // Before #41 this reached the chat revoke, which the library refuses for
    // status@broadcast, and the edge read `Unavailable`.
    let delete = messaging
        .delete_message(pb::DeleteMessageRequest {
            account: account.clone(),
            target: Some(pb::MessageKey {
                chat: Some(pb::Jid {
                    value: "status@broadcast".to_string(),
                }),
                id: "3EB0STATUS".to_string(),
                from_me: true,
                participant: None,
            }),
            for_everyone: true,
        })
        .await
        .expect_err("a status is not revoked through DeleteMessage");
    assert_eq!(delete.code(), tonic::Code::InvalidArgument);
    assert!(
        delete.message().contains("RevokeStatus"),
        "the refusal must name the RPC to use: {}",
        delete.message()
    );

    let shapes = [
        ("", vec!["5511999000111@s.whatsapp.net".to_string()]),
        ("3EB0STATUS", vec![]),
        ("3EB0STATUS", vec![String::new()]),
    ];
    for (message_id, recipients) in shapes {
        let revoke = messaging
            .revoke_status(pb::RevokeStatusRequest {
                account: account.clone(),
                message_id: message_id.to_string(),
                recipients: recipients
                    .iter()
                    .map(|value| pb::Jid {
                        value: value.clone(),
                    })
                    .collect(),
            })
            .await
            .expect_err("a malformed revoke is refused");
        assert_eq!(
            revoke.code(),
            tonic::Code::InvalidArgument,
            "id {message_id:?}, recipients {recipients:?}"
        );
    }

    accounts
        .delete_account(account_ref(&created.uuid))
        .await
        .expect("delete");
}
