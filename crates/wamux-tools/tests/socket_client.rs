//! #64: the one shared connection to the daemon, exercised over a real socket.

use std::time::Duration;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux_tools::socket_client::{ClientError, account_ref, connect_uds, wait_connected};

mod support;
use support::TestDaemon;

#[tokio::test]
async fn connect_uds_reaches_the_daemon_and_rpcs_work() {
    let daemon = TestDaemon::start().await;
    daemon
        .registry
        .create_account(Some("tools-64-listed"))
        .await
        .unwrap();

    let channel = connect_uds(&daemon.socket_str()).await.unwrap();
    let accounts = AccountServiceClient::new(channel)
        .list_accounts(pb::ListAccountsRequest {})
        .await
        .unwrap()
        .into_inner()
        .accounts;
    assert!(
        accounts.iter().any(|a| a.external_ref == "tools-64-listed"),
        "{accounts:?}"
    );
}

#[tokio::test]
async fn connect_uds_to_a_missing_socket_is_an_error_naming_the_path() {
    let err = connect_uds("/tmp/wmx64-no-such-dir/wamux.sock")
        .await
        .unwrap_err();
    assert!(matches!(err, ClientError::Connect { .. }), "{err:?}");
    assert!(
        err.to_string()
            .contains("/tmp/wmx64-no-such-dir/wamux.sock"),
        "{err}"
    );
}

#[test]
fn account_ref_is_by_external_ref() {
    assert_eq!(
        account_ref("pessoal"),
        pb::AccountRef {
            r#ref: Some(pb::account_ref::Ref::ExternalRef("pessoal".into()))
        }
    );
}

#[tokio::test]
async fn wait_connected_on_an_unknown_account_fails_instead_of_carrying_on() {
    let daemon = TestDaemon::start().await;
    let channel = connect_uds(&daemon.socket_str()).await.unwrap();
    let mut account = AccountServiceClient::new(channel);
    let err = wait_connected(
        &mut account,
        &account_ref("tools-64-nobody"),
        Duration::from_secs(5),
    )
    .await
    .unwrap_err();
    match err {
        ClientError::Rpc { status, .. } => assert_eq!(status.code(), tonic::Code::NotFound),
        other => panic!("expected the daemon's NotFound, got {other:?}"),
    }
}
