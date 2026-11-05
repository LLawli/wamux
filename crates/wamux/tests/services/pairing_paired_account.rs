//! #253: a pairing on an account that holds a stored `pn` is refused. The
//! library logs such a device in and never asks for a QR or a code, and the
//! stream only forwards pairing events, so it would stay silent. That covers a
//! logged-out account too: `Logout` keeps the `pn`, the next connect logs in,
//! and WhatsApp answers `<failure reason="401">`, measured 2026-11-05.
//!
//! The account is logged in against `MockWaServer`, which accepts any login, so
//! these tests prove the refusal and not the 401. Honors `WAMUX_TEST_ENGINE`.

use crate::common;
use std::sync::Arc;
use std::time::Duration;

use tokio_stream::StreamExt;
use tonic::transport::Channel;
use tonic::{Code, Status};
use wacore::types::events::{Event, PairingQrCode};
use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::state::AccountHandle;
use wamux::stress::MockWaServer;

use common::mock_wire::account_ref;

/// Long enough for a loaded CI runner, short enough to fail fast.
const WAIT: Duration = Duration::from_secs(10);
const QR: &str = "2@pairing-paired-account-qr";

type Pairing = tonic::Streaming<pb::PairingUpdate>;

struct Fixture {
    _mock: MockWaServer,
    logged: common::LoggedIn,
    accounts: AccountServiceClient<Channel>,
}

async fn fixture(test: &str) -> Fixture {
    let mock = MockWaServer::start().await.expect("start mock");
    let prefix = common::test_prefix("pairing_paired_account", test);
    let logged = common::logged_in_client(&mock, &prefix).await;
    let channel = common::serve_registry(logged.registry.clone()).await;
    Fixture {
        _mock: mock,
        logged,
        accounts: AccountServiceClient::new(channel),
    }
}

fn reference_of(handle: &AccountHandle) -> pb::AccountRef {
    account_ref(&handle.uuid.to_string())
}

impl Fixture {
    fn account(&self) -> pb::AccountRef {
        reference_of(&self.logged.handle)
    }

    /// A hang past `WAIT` is the bug itself, so it fails here.
    async fn pair_with_qr(&mut self, handle: &AccountHandle) -> Result<Pairing, Status> {
        let request = pb::PairWithQrRequest {
            account: Some(reference_of(handle)),
            backfill_history: false,
        };
        let call = tokio::time::timeout(WAIT, self.accounts.pair_with_qr(request));
        let answer = call.await.expect("PairWithQr did not answer");
        answer.map(tonic::Response::into_inner)
    }

    async fn pair_with_code(&mut self, handle: &AccountHandle) -> Result<Pairing, Status> {
        let request = pb::PairWithCodeRequest {
            account: Some(reference_of(handle)),
            phone_number: "5511999999999".to_string(),
            custom_code: None,
            backfill_history: false,
        };
        let call = tokio::time::timeout(WAIT, self.accounts.pair_with_code(request));
        let answer = call.await.expect("PairWithCode did not answer");
        answer.map(tonic::Response::into_inner)
    }

    async fn state(&mut self) -> pb::ConnectionState {
        let status = self.accounts.get_account_status(self.account()).await;
        status.expect("GetAccountStatus").into_inner().state()
    }
}

/// The failed call, or a panic naming what the client got instead.
fn refusal(answer: Result<Pairing, Status>) -> Status {
    match answer {
        Err(status) => status,
        Ok(_) => panic!("the call opened a stream that will never carry anything"),
    }
}

#[tokio::test]
async fn pair_with_qr_refuses_a_running_paired_account() {
    let mut fixture = fixture("running_qr").await;
    let handle = fixture.logged.handle.clone();
    let answer = fixture.pair_with_qr(&handle).await;
    assert_eq!(refusal(answer).code(), Code::FailedPrecondition);
    fixture.logged.cleanup().await;
}

#[tokio::test]
async fn pair_with_code_refuses_a_running_paired_account() {
    let mut fixture = fixture("running_code").await;
    let handle = fixture.logged.handle.clone();
    let answer = fixture.pair_with_code(&handle).await;
    assert_eq!(refusal(answer).code(), Code::FailedPrecondition);
    fixture.logged.cleanup().await;
}

/// The refusal comes before the connect: a stopped account must not come up
/// logged in as a side effect of a call that failed.
#[tokio::test]
async fn pair_with_qr_refuses_a_stopped_paired_account() {
    let mut fixture = fixture("stopped").await;
    let handle = fixture.logged.handle.clone();
    let stopped = fixture.accounts.disconnect_account(fixture.account()).await;
    stopped.expect("DisconnectAccount");

    let answer = fixture.pair_with_qr(&handle).await;

    assert_eq!(refusal(answer).code(), Code::FailedPrecondition);
    assert!(!handle.is_running().await, "the refusal started the bot");
    assert_eq!(fixture.state().await, pb::ConnectionState::Disconnected);
    fixture.logged.cleanup().await;
}

/// `Logout` keeps the `pn`, so this is the same refusal, and connecting would
/// overwrite LOGGED_OUT only to get it back from a 401.
#[tokio::test]
async fn pair_with_qr_refuses_a_logged_out_account() {
    let mut fixture = fixture("logged_out").await;
    let handle = fixture.logged.handle.clone();
    let logout = fixture.accounts.logout(fixture.account()).await;
    logout.expect("Logout");

    let answer = fixture.pair_with_qr(&handle).await;

    assert_eq!(refusal(answer).code(), Code::FailedPrecondition);
    assert!(!handle.is_running().await, "the refusal started the bot");
    assert_eq!(fixture.state().await, pb::ConnectionState::LoggedOut);
    fixture.logged.cleanup().await;
}

/// The client has no other way to learn that the account has to be replaced.
#[tokio::test]
async fn the_refusal_names_the_way_out() {
    let mut fixture = fixture("way_out").await;
    let handle = fixture.logged.handle.clone();
    let answer = fixture.pair_with_qr(&handle).await;
    let status = refusal(answer);
    assert!(
        status.message().contains("DeleteAccount"),
        "the message does not say what to do: {:?}",
        status.message()
    );
    fixture.logged.cleanup().await;
}

/// The way out the refusal names: the same `external_ref` on a new account has
/// a device of its own, with no `pn`, so it pairs.
#[tokio::test]
async fn a_deleted_and_recreated_account_pairs_again() {
    let mut fixture = fixture("recreated").await;
    let handle = fixture.logged.handle.clone();
    let name = handle
        .external_ref
        .clone()
        .expect("the test account's name");
    let deleted = fixture.accounts.delete_account(fixture.account()).await;
    deleted.expect("DeleteAccount");
    let registry = fixture.logged.registry.clone();
    let created = registry.create_account(Some(&name)).await;
    let recreated: Arc<AccountHandle> = created.expect("create the account again");

    let answer = fixture.pair_with_qr(&recreated).await;
    let mut stream = answer.expect("a recreated account must pair");
    recreated
        .publisher
        .publish_lib(Arc::new(Event::PairingQrCode(
            PairingQrCode::builder()
                .code(QR.to_string())
                .timeout(Duration::from_secs(60))
                .build(),
        )));

    let update = tokio::time::timeout(WAIT, stream.next()).await;
    let update = update.expect("no QR within the wait");
    let update = update.expect("the stream ended before the QR");
    assert_eq!(
        update.expect("pairing stream").event,
        Some(pb::pairing_update::Event::QrCode(QR.to_string()))
    );
    let deleted = registry.delete(&recreated).await;
    deleted.expect("delete the recreated account");
}
