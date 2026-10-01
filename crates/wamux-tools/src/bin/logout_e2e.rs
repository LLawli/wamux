//! E2E for real Logout (server-side device unlink) over the socket:
//!   1. ConnectAccount (logout needs a live connection).
//!   2. Logout -> sends the RemoveCompanionDevice IQ; the device should vanish
//!      from the phone's "linked devices" list.
//!   3. GetAccountStatus -> expect DISCONNECTED. Logout keeps the account row +
//!      local keys (re-pairable); only DeleteAccount wipes state.
//!
//! Env: WAMUX_REF (required), WAMUX_SOCKET_PATH.

use std::process::ExitCode;
use std::time::Duration;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux_tools::live_env::{account_ref_from, process_env, socket_path_from};
use wamux_tools::report::Report;
use wamux_tools::socket_client::{account_ref, connect_uds, wait_connected};

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let external_ref: String = account_ref_from(&process_env)?;
    let socket: String = socket_path_from(&process_env)?;
    let channel = connect_uds(&socket).await?;
    let mut account = AccountServiceClient::new(channel);
    let acct = account_ref(&external_ref);
    let mut report = Report::new();

    println!("connecting '{external_ref}' ...");
    let jid = wait_connected(&mut account, &acct, Duration::from_secs(30)).await?;
    report.pass("Account.ConnectAccount", format!("connected as {jid}"));

    println!("calling Logout (real RemoveCompanionDevice unlink) ...");
    if report
        .accepted_rpc("Account.Logout", account.logout(acct.clone()).await)
        .is_some()
    {
        check_disconnected(&mut account, &mut report, &acct).await;
    }
    println!(
        "\n=== CONFIRM ON YOUR PHONE ===\n\
         WhatsApp -> Settings -> Linked devices: the wamux device should be GONE.\n\
         The account row + local keys are kept (re-pairable via QR)."
    );
    Ok(report.finish())
}

/// The state after Logout is the asserted value: DISCONNECTED, not merely
/// "the RPC answered".
async fn check_disconnected(
    account: &mut AccountServiceClient<tonic::transport::Channel>,
    report: &mut Report,
    acct: &pb::AccountRef,
) {
    let after = match account.get_account_status(acct.clone()).await {
        Ok(resp) => resp.into_inner(),
        Err(status) => return report.fail("Account.GetAccountStatus", status.to_string()),
    };
    let state = pb::ConnectionState::try_from(after.state)
        .map(|s| format!("{s:?}"))
        .unwrap_or_else(|_| after.state.to_string());
    report.verify(
        "Account.GetAccountStatus after Logout",
        after.state == pb::ConnectionState::Disconnected as i32,
        format!("post-logout state = {state}, expected Disconnected"),
    );
}
