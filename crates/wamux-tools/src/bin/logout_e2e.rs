//! E2E for real Logout (server-side device unlink) over the socket:
//!   1. ConnectAccount (logout needs a live connection).
//!   2. GetAccountStatus -> show jid/state.
//!   3. Logout -> sends the RemoveCompanionDevice IQ; the device should vanish
//!      from the phone's "linked devices" list.
//!   4. GetAccountStatus -> expect DISCONNECTED. Logout keeps the account row +
//!      local keys (re-pairable); only DeleteAccount wipes state.
//!
//! Usage: logout_e2e [socket] [external_ref]   (default /tmp/wamux.sock pair-socket)

use std::time::Duration;

use hyper_util::rt::TokioIo;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket = arg(1, "/tmp/wamux.sock");
    let external_ref = arg(2, "pair-socket");

    let channel = connect_uds(socket).await?;
    let mut account = AccountServiceClient::new(channel);
    let acct = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(external_ref.clone())),
    };

    println!("connecting '{external_ref}' ...");
    account
        .connect_account(pb::ConnectAccountRequest {
            account: Some(acct.clone()),
            backfill_history: false,
        })
        .await?;
    let mut connected = false;
    for _ in 0..100 {
        if let Ok(s) = account.get_account_status(acct.clone()).await {
            let s = s.into_inner();
            if s.state == pb::ConnectionState::Connected as i32 {
                let jid = s.jid.map(|j| j.value).unwrap_or_default();
                println!("connected ✅  jid={jid}  push_name={:?}", s.push_name);
                connected = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    anyhow::ensure!(connected, "account did not reach CONNECTED");

    println!("calling Logout (real RemoveCompanionDevice unlink) ...");
    account.logout(acct.clone()).await?;
    println!("Logout RPC returned OK ✅");

    let after = account.get_account_status(acct.clone()).await?.into_inner();
    let state = pb::ConnectionState::try_from(after.state)
        .map(|s| format!("{s:?}"))
        .unwrap_or_else(|_| after.state.to_string());
    println!("post-logout state = {state}");
    println!(
        "\n=== CONFIRM ON YOUR PHONE ===\n\
         WhatsApp -> Settings -> Linked devices: the wamux device should be GONE.\n\
         The account row + local keys are kept (re-pairable via QR)."
    );
    Ok(())
}

fn arg(n: usize, default: &str) -> String {
    std::env::args()
        .nth(n)
        .unwrap_or_else(|| default.to_string())
}

async fn connect_uds(path: String) -> anyhow::Result<Channel> {
    Ok(Endpoint::try_from("http://[::1]:50051")?
        .connect_with_connector(service_fn(move |_: Uri| {
            let path = path.clone();
            async move {
                Ok::<_, std::io::Error>(TokioIo::new(tokio::net::UnixStream::connect(path).await?))
            }
        }))
        .await?)
}
