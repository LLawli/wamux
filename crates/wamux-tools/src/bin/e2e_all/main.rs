//! Comprehensive E2E across the whole wamux socket API, non-destructive subset
//! (#64). Prints one `[PASS]` / `[ACCEPTED]` / `[FAIL]` line per check and a
//! summary, and exits non-zero unless there is at least one pass and no fail.
//! The destructive half (profile, groups) is `e2e_destructive`.
//!
//! Env: WAMUX_REF          the paired account to act as (required)
//!      WAMUX_LIVE_DEST    the one chat it sends to (required, @s.whatsapp.net)
//!      WAMUX_DELIVERY_SECS  how long a send waits for its `delivered` receipt
//!      WAMUX_E2E_INBOUND=1  opt in to the 60s window where a human sends this
//!                         account a text and an image, then DownloadMedia
//!      WAMUX_SOCKET_PATH
//!
//! Requires the daemon running with the account already paired. The core
//! relays to the JID verbatim (no routing, CLAUDE.md); a send passes only when
//! its fan-out reached the phone and `delivered` arrives, never on the ack.

mod inbound;
mod lifecycle;
mod reads;
mod sends;

use std::process::ExitCode;
use std::time::Duration;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;
use wamux_tools::delivery::EventTap;
use wamux_tools::live_env::{
    account_ref_from, delivery_window_from, inbound_requested_from, live_dest_from, process_env,
    refuse_own_number, socket_path_from,
};
use wamux_tools::report::Report;
use wamux_tools::socket_client::{ClientError, account_ref, connect_uds, wait_connected};

const CONNECT_WITHIN: Duration = Duration::from_secs(30);

/// What every phase after the connect needs: who acts, where it sends, and
/// the subscription that sees the receipts of those sends.
pub struct E2eCtx {
    pub acct: pb::AccountRef,
    pub dest: String,
    pub window: Duration,
    pub tap: EventTap,
}

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    // The whole config first: nothing is opened until every variable is valid.
    let external_ref: String = account_ref_from(&process_env)?;
    let dest: String = live_dest_from(&process_env)?;
    let window: Duration = delivery_window_from(&process_env)?;
    let inbound: bool = inbound_requested_from(&process_env);
    let socket: String = socket_path_from(&process_env)?;

    let channel = connect_uds(&socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut report = Report::new();
    lifecycle::run_admin_and_temp_account(&channel, &mut report).await;

    let acct = account_ref(&external_ref);
    let connected = wait_connected(&mut account, &acct, CONNECT_WITHIN).await;
    let own = match connected {
        Ok(jid) => jid,
        Err(error) => return Ok(connect_failed(report, &error)),
    };
    report.pass("Account.ConnectAccount", format!("connected as {own}"));
    refuse_own_number(&own, &dest)?;

    // Subscribe BEFORE the first send: its receipt can land at any moment.
    let mut events = EventServiceClient::new(channel.clone());
    let sub = pb::SubscribeRequest {
        selector: Some(pb::subscribe_request::Selector::Account(acct.clone())),
        replay_from_ring: 0,
    };
    let Some(stream) =
        report.accepted_rpc("Event.SubscribeEvents", events.subscribe_events(sub).await)
    else {
        return Ok(report.finish());
    };
    let ctx = E2eCtx {
        acct,
        dest,
        window,
        tap: EventTap::spawn(stream.into_inner()),
    };
    reads::run_contact_and_group_reads(&channel, &mut report, &ctx).await;
    let mut messaging = MessagingServiceClient::new(channel.clone());
    let text_key = sends::run_sends(&mut messaging, &mut report, &ctx).await;
    if inbound {
        inbound::run_inbound_window(&channel, &mut report, &ctx).await;
    }
    sends::revoke_text(&mut messaging, &mut report, &ctx, text_key).await;
    Ok(report.finish())
}

/// A failed connect ends the run at once: every later check would be about an
/// account that is not there, and nothing may be sent from it.
fn connect_failed(mut report: Report, error: &ClientError) -> ExitCode {
    report.fail("Account.ConnectAccount", error.to_string());
    report.finish()
}
