//! The checks that need no WhatsApp account: the metrics endpoint and the
//! create/list/status/delete lifecycle of a THROWAWAY account (reversible).

use std::time::{SystemTime, UNIX_EPOCH};

use tonic::transport::Channel;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::admin_service_client::AdminServiceClient;
use wamux_tools::report::Report;
use wamux_tools::socket_client::account_ref;

pub async fn run_admin_and_temp_account(channel: &Channel, report: &mut Report) {
    let mut admin = AdminServiceClient::new(channel.clone());
    match admin.get_metrics(pb::Empty {}).await {
        Ok(r) => {
            let bytes = r.into_inner().prometheus.len();
            report.verify("Admin.GetMetrics", bytes > 0, format!("{bytes} bytes"));
        }
        Err(e) => report.fail("Admin.GetMetrics", e.to_string()),
    }
    let mut account = AccountServiceClient::new(channel.clone());
    let tmp_ref = format!("e2e-tmp-{}", nanos());
    let Some(uuid) = create_temp(&mut account, report, &tmp_ref).await else {
        return;
    };
    check_listed(&mut account, report, &tmp_ref, true, "Account.ListAccounts").await;
    check_status(&mut account, report, &tmp_ref, &uuid).await;
    delete_temp(&mut account, report, &tmp_ref).await;
}

async fn create_temp(
    account: &mut AccountServiceClient<Channel>,
    report: &mut Report,
    tmp_ref: &str,
) -> Option<String> {
    let created = account
        .create_account(pb::CreateAccountRequest {
            external_ref: Some(tmp_ref.to_string()),
        })
        .await;
    match created {
        Ok(r) => {
            let uuid: String = r.into_inner().uuid;
            report.verify(
                "Account.CreateAccount",
                !uuid.is_empty(),
                format!("uuid={uuid}"),
            );
            Some(uuid).filter(|uuid| !uuid.is_empty())
        }
        Err(e) => {
            report.fail("Account.CreateAccount", e.to_string());
            None
        }
    }
}

/// Whether the account list holds `tmp_ref`, asserted against `want`.
async fn check_listed(
    account: &mut AccountServiceClient<Channel>,
    report: &mut Report,
    tmp_ref: &str,
    want: bool,
    name: &str,
) {
    match account.list_accounts(pb::ListAccountsRequest {}).await {
        Ok(r) => {
            let listed = r.into_inner().accounts;
            let found = listed.iter().any(|a| a.external_ref == tmp_ref);
            report.verify(
                name,
                found == want,
                format!(
                    "{} accounts; {tmp_ref} listed={found}, expected {want}",
                    listed.len()
                ),
            );
        }
        Err(e) => report.fail(name, e.to_string()),
    }
}

async fn check_status(
    account: &mut AccountServiceClient<Channel>,
    report: &mut Report,
    tmp_ref: &str,
    uuid: &str,
) {
    match account.get_account_status(account_ref(tmp_ref)).await {
        Ok(r) => {
            let status = r.into_inner();
            report.verify(
                "Account.GetAccountStatus(tmp)",
                status.uuid == uuid,
                format!("uuid={} state={}", status.uuid, status.state),
            );
        }
        Err(e) => report.fail("Account.GetAccountStatus(tmp)", e.to_string()),
    }
}

/// The delete is judged by value: the account is no longer listed afterwards.
async fn delete_temp(
    account: &mut AccountServiceClient<Channel>,
    report: &mut Report,
    tmp_ref: &str,
) {
    match account.delete_account(account_ref(tmp_ref)).await {
        Ok(_) => {
            check_listed(
                account,
                report,
                tmp_ref,
                false,
                "Account.DeleteAccount(tmp)",
            )
            .await;
        }
        Err(e) => report.fail("Account.DeleteAccount(tmp)", e.to_string()),
    }
}

fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}
