//! The one way a wamux-tools binary reaches the daemon: gRPC over its Unix
//! socket (#64). There is no TCP listener; the URI below is only a placeholder
//! tonic requires, and the connector dials the socket path instead.

use std::time::Duration;

use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;
use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("cannot reach the wamux socket at {path}: {source}")]
    Connect {
        path: String,
        #[source]
        source: tonic::transport::Error,
    },
    #[error("account {account} did not reach CONNECTED with a jid within {waited:?}")]
    NotConnected { account: String, waited: Duration },
    #[error("{rpc} failed: {status}")]
    Rpc {
        rpc: &'static str,
        status: tonic::Status,
    },
}

/// Open a channel to the daemon's socket at `path`.
pub async fn connect_uds(path: &str) -> Result<Channel, ClientError> {
    let owned: String = path.to_string();
    let connector = service_fn(move |_: Uri| {
        let socket: String = owned.clone();
        async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(socket).await?)) }
    });
    let to_error = |source: tonic::transport::Error| ClientError::Connect {
        path: path.to_string(),
        source,
    };
    Endpoint::try_from("http://[::1]:50051")
        .map_err(to_error)?
        .connect_with_connector(connector)
        .await
        .map_err(to_error)
}

/// An `AccountRef` by external_ref, the name every binary is configured with.
pub fn account_ref(external: &str) -> pb::AccountRef {
    pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(external.to_string())),
    }
}

/// How often the status is polled while waiting for CONNECTED.
const CONNECT_POLL: Duration = Duration::from_millis(300);

/// Ask the daemon to connect the account, then wait until it reports
/// CONNECTED with a jid, and return that jid. Fails instead of carrying on:
/// a binary that continued without a connection would report on nothing.
pub async fn wait_connected(
    account: &mut AccountServiceClient<Channel>,
    acct: &pb::AccountRef,
    within: Duration,
) -> Result<String, ClientError> {
    wait_connected_with(account, acct, within, false).await
}

/// `wait_connected`, choosing whether the phone's history dump is processed
/// (`backfill_history`, needed by the history probes; the default is off).
pub async fn wait_connected_with(
    account: &mut AccountServiceClient<Channel>,
    acct: &pb::AccountRef,
    within: Duration,
    backfill_history: bool,
) -> Result<String, ClientError> {
    let request = pb::ConnectAccountRequest {
        account: Some(acct.clone()),
        backfill_history,
    };
    let rpc_error = |rpc: &'static str| move |status| ClientError::Rpc { rpc, status };
    account
        .connect_account(request)
        .await
        .map_err(rpc_error("ConnectAccount"))?;
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let status = account
            .get_account_status(acct.clone())
            .await
            .map_err(rpc_error("GetAccountStatus"))?
            .into_inner();
        let jid: String = status.jid.map(|j| j.value).unwrap_or_default();
        if status.state == pb::ConnectionState::Connected as i32 && !jid.is_empty() {
            return Ok(jid);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(ClientError::NotConnected {
                account: describe(acct),
                waited: within,
            });
        }
        tokio::time::sleep(CONNECT_POLL).await;
    }
}

fn describe(acct: &pb::AccountRef) -> String {
    match &acct.r#ref {
        Some(pb::account_ref::Ref::ExternalRef(name)) => name.clone(),
        Some(pb::account_ref::Ref::Uuid(uuid)) => uuid.clone(),
        None => "<unnamed>".to_string(),
    }
}
