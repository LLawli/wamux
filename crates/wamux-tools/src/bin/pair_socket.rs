//! Pairing validator that drives the REAL wamux socket (not the library):
//! connects to the daemon's Unix socket via gRPC, calls AccountService
//! CreateAccount + PairWithQr, renders the QR to /tmp/wamux-qr-<ref>.png and
//! opens it with xdg-open. On pairing, sends a self-message via MessagingService.
//!
//! Env: WAMUX_REF (required, the account's stable name; pass one per phone you
//! pair, since the daemon multiplexes many), WAMUX_SOCKET_PATH.
//! Requires the wamux daemon to be running on that socket.

use std::path::PathBuf;
use std::process::ExitCode;

use tonic::transport::Channel;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;
use wamux_tools::live_env::{account_ref_from, process_env, socket_path_from};
use wamux_tools::qr::{ascii_qr, open_in_viewer, write_qr_png};
use wamux_tools::report::Report;
use wamux_tools::socket_client::{account_ref, connect_uds};

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let external_ref: String = account_ref_from(&process_env)?;
    let socket_path: String = socket_path_from(&process_env)?;
    // One PNG per account: pairing two phones in parallel would otherwise have
    // the second render overwrite the first's still-unscanned QR.
    let qr_png = PathBuf::from(format!("/tmp/wamux-qr-{external_ref}.png"));
    let channel = connect_uds(&socket_path).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut messaging = MessagingServiceClient::new(channel);
    let mut report = Report::new();

    create_if_missing(&mut account, &mut report, &external_ref).await;
    let acct = account_ref(&external_ref);
    println!("calling PairWithQr over the socket (account '{external_ref}') ...");
    let stream = account
        .pair_with_qr(pb::PairWithQrRequest {
            account: Some(acct.clone()),
            backfill_history: false,
        })
        .await;
    match stream {
        Ok(response) => {
            let jid = watch_pairing(response.into_inner(), &qr_png, &mut report).await;
            if let Some(jid) = jid {
                send_self(&mut messaging, &mut report, acct, jid).await;
            }
        }
        Err(status) => report.fail("Account.PairWithQr", status.to_string()),
    }
    Ok(report.finish())
}

/// PairWithQr resolves by external_ref, so the account must exist; an
/// existing one is reused (the daemon refuses a duplicate ref).
async fn create_if_missing(
    account: &mut AccountServiceClient<Channel>,
    report: &mut Report,
    external_ref: &str,
) {
    let created = account
        .create_account(pb::CreateAccountRequest {
            external_ref: Some(external_ref.to_string()),
        })
        .await;
    match created {
        Ok(resp) => {
            let uuid: String = resp.into_inner().uuid;
            report.verify(
                "Account.CreateAccount",
                !uuid.is_empty(),
                format!("uuid={uuid}"),
            );
        }
        Err(status) => println!("create_account: {} (reusing existing)", status.message()),
    }
}

/// Render each QR until the phone pairs; returns the paired jid.
async fn watch_pairing(
    mut stream: tonic::Streaming<pb::PairingUpdate>,
    qr_png: &std::path::Path,
    report: &mut Report,
) -> Option<String> {
    let mut opened = false;
    loop {
        let update = match stream.message().await {
            Ok(Some(update)) => update,
            Ok(None) => {
                report.fail("Account.PairWithQr", "stream ended before the phone paired");
                return None;
            }
            Err(status) => {
                report.fail("Account.PairWithQr", status.to_string());
                return None;
            }
        };
        match update.event {
            Some(pb::pairing_update::Event::QrCode(code)) => {
                show_qr(&code, qr_png, &mut opened);
            }
            Some(pb::pairing_update::Event::PairCode(code)) => println!("pair code: {code}"),
            Some(pb::pairing_update::Event::Paired(info)) => {
                let jid: String = info.jid.map(|j| j.value).unwrap_or_default();
                report.verify(
                    "Account.PairWithQr",
                    !jid.is_empty(),
                    format!("paired as {jid}"),
                );
                return Some(jid).filter(|jid| !jid.is_empty());
            }
            Some(pb::pairing_update::Event::Error(err)) => {
                report.fail("Account.PairWithQr", err.message);
                return None;
            }
            None => {}
        }
    }
}

fn show_qr(code: &str, qr_png: &std::path::Path, opened: &mut bool) {
    match write_qr_png(code, qr_png) {
        Err(e) => eprintln!("[qr] render failed: {e}"),
        Ok(()) => {
            println!("[qr] new QR -> {}", qr_png.display());
            if !*opened {
                open_in_viewer(qr_png);
                *opened = true;
            }
        }
    }
    match ascii_qr(code) {
        Ok(text) => println!("{text}"),
        Err(e) => eprintln!("[qr] {e}"),
    }
}

/// A note to self: a self-chat has no delivery receipt and no fan-out
/// (CLAUDE.md), so the send can only ever be `accepted`.
async fn send_self(
    messaging: &mut MessagingServiceClient<Channel>,
    report: &mut Report,
    acct: pb::AccountRef,
    jid: String,
) {
    let request = pb::SendTextRequest {
        account: Some(acct),
        to: Some(pb::Jid { value: jid }),
        text: "wamux: pareamento via socket + envio OK".to_string(),
        ..Default::default()
    };
    let sent = messaging.send_text(request).await;
    report.accepted_rpc("Messaging.SendText(self)", sent);
}
