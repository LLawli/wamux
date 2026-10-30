//! #77: the `SO_PEERCRED` allowlist, over the real socket. The peer is this
//! test process, so its effective uid and gid are what the kernel reports.

use crate::common;
use std::sync::Arc;

use nix::unistd::{getegid, geteuid};
use tonic::Code;
use tonic::transport::Channel;

use wamux::config::Config;
use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::state::{AccountRegistry, RegistryTuning};

fn our_uid() -> u32 {
    geteuid().as_raw()
}

fn our_gid() -> u32 {
    getegid().as_raw()
}

async fn serve(allowed_uids: Vec<u32>, allowed_gids: Vec<u32>) -> Channel {
    let registry = Arc::new(AccountRegistry::new(
        common::test_engine().await,
        RegistryTuning::with_ring(64),
    ));
    let config = Config {
        allowed_uids,
        allowed_gids,
        ..Config::default()
    };
    common::serve_registry_with(registry, config).await
}

async fn list_accounts(channel: Channel) -> Result<pb::ListAccountsResponse, tonic::Status> {
    AccountServiceClient::new(channel)
        .list_accounts(pb::ListAccountsRequest {})
        .await
        .map(tonic::Response::into_inner)
}

#[tokio::test]
async fn an_unlisted_uid_gets_permission_denied() {
    let channel = serve(
        vec![our_uid().wrapping_add(1)],
        vec![our_gid().wrapping_add(1)],
    )
    .await;
    let status = list_accounts(channel)
        .await
        .expect_err("a peer in neither list must be refused");
    assert_eq!(status.code(), Code::PermissionDenied, "{status:?}");
    assert_eq!(status.message(), "peer not allowed");
}

#[tokio::test]
async fn the_peer_uid_in_allowed_uids_is_served() {
    let channel = serve(vec![our_uid()], vec![]).await;
    list_accounts(channel)
        .await
        .expect("the listed uid is served");
}

#[tokio::test]
async fn the_peer_gid_in_allowed_gids_is_served() {
    let channel = serve(vec![our_uid().wrapping_add(1)], vec![our_gid()]).await;
    list_accounts(channel)
        .await
        .expect("the listed gid is served though the uid is not listed");
}

#[tokio::test]
async fn no_allowlist_serves_everyone() {
    let channel = serve(vec![], vec![]).await;
    list_accounts(channel)
        .await
        .expect("no allowlist, as before #77");
}
