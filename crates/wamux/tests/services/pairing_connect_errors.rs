//! #250: a pairing whose connect fails says so. The stream only forwards
//! pairing events and a failed connect publishes none, so an RPC that handed
//! the stream back first left the client waiting with no update and no error.
//!
//! Two ways to fail, neither reaching a network: the connection budget, and a
//! non-loopback upstream, which the `stress` build's guard refuses inside
//! `build_bot`. The upstream that does get connected to accepts and never
//! answers, so the connect stays in its handshake and nothing ends the stream
//! but the test. Honors `WAMUX_TEST_ENGINE`.

use crate::common;
use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_stream::StreamExt;
use tonic::transport::Channel;
use tonic::{Code, Status};
use wacore::types::events::{Event, PairingQrCode};
use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::state::{AccountHandle, AccountRegistry, RegistryTuning};

use common::mock_wire::account_ref;

/// Long enough for a loaded CI runner, short enough to fail fast.
const WAIT: Duration = Duration::from_secs(10);
const QR: &str = "2@pairing-connect-errors-qr";
/// Refused by the `stress` guard before anything is dialed.
const REFUSED_UPSTREAM: &str = "ws://example.invalid:1";

type Pairing = tonic::Streaming<pb::PairingUpdate>;

struct Fixture {
    _upstream: TcpListener,
    registry: Arc<AccountRegistry>,
    accounts: AccountServiceClient<Channel>,
    prefix: String,
}

async fn fixture(test: &str, ws_url: Option<&str>, budget: usize) -> Fixture {
    let upstream = TcpListener::bind("127.0.0.1:0").expect("bind the silent upstream");
    let port = upstream.local_addr().expect("upstream address").port();
    let engine = common::test_engine().await;
    let prefix = common::test_prefix("pairing_connect_errors", test);
    common::sweep_orphans(&engine, &prefix).await;
    let tuning = RegistryTuning {
        ws_url_override: Some(
            ws_url.map_or_else(|| format!("ws://127.0.0.1:{port}"), str::to_string),
        ),
        max_connected_accounts: budget,
        ..RegistryTuning::default()
    };
    let registry = Arc::new(AccountRegistry::new(engine, tuning));
    let accounts = AccountServiceClient::new(common::serve_registry(registry.clone()).await);
    Fixture {
        _upstream: upstream,
        registry,
        accounts,
        prefix,
    }
}

fn account_of(handle: &AccountHandle) -> Option<pb::AccountRef> {
    Some(account_ref(&handle.uuid.to_string()))
}

impl Fixture {
    async fn new_account(&self) -> Arc<AccountHandle> {
        let name = format!("{}{}", self.prefix, uuid::Uuid::new_v4());
        let created = self.registry.create_account(Some(&name)).await;
        created.expect("create account")
    }

    /// An account holding the only budget slot.
    async fn connected_account(&self) -> Arc<AccountHandle> {
        let handle = self.new_account().await;
        let connected = self.registry.connect(&handle, None, true).await;
        connected.expect("connect the first account");
        handle
    }

    /// `Err` is the point of this suite: the call itself must say the connect
    /// failed. A hang past `WAIT` is a failure of its own.
    async fn pair_with_qr(&mut self, handle: &AccountHandle) -> Result<Pairing, Status> {
        let request = pb::PairWithQrRequest {
            account: account_of(handle),
            backfill_history: false,
        };
        let call = tokio::time::timeout(WAIT, self.accounts.pair_with_qr(request));
        let answer = call.await.expect("PairWithQr did not answer");
        answer.map(tonic::Response::into_inner)
    }

    async fn pair_with_code(&mut self, handle: &AccountHandle) -> Result<Pairing, Status> {
        let request = pb::PairWithCodeRequest {
            account: account_of(handle),
            phone_number: "5511999999999".to_string(),
            custom_code: None,
            backfill_history: false,
        };
        let call = tokio::time::timeout(WAIT, self.accounts.pair_with_code(request));
        let answer = call.await.expect("PairWithCode did not answer");
        answer.map(tonic::Response::into_inner)
    }

    async fn state_of(&mut self, handle: &AccountHandle) -> pb::ConnectionState {
        let reference = account_ref(&handle.uuid.to_string());
        let status = self.accounts.get_account_status(reference).await;
        status.expect("GetAccountStatus").into_inner().state()
    }

    async fn delete(&self, handle: &Arc<AccountHandle>) {
        let deleted = self.registry.delete(handle).await;
        deleted.expect("delete the account");
    }
}

/// The code of a failed call, or a panic naming what the client got instead.
fn refused(answer: Result<Pairing, Status>) -> Code {
    match answer {
        Err(status) => status.code(),
        Ok(_) => panic!("the call opened a stream that will never carry anything"),
    }
}

#[tokio::test]
async fn pair_with_qr_reports_a_full_connection_budget() {
    let mut fixture = fixture("budget_qr", None, 1).await;
    let holder = fixture.connected_account().await;
    let refused_account = fixture.new_account().await;

    let answer = fixture.pair_with_qr(&refused_account).await;

    assert_eq!(refused(answer), Code::ResourceExhausted);
    fixture.delete(&refused_account).await;
    fixture.delete(&holder).await;
}

#[tokio::test]
async fn pair_with_code_reports_a_full_connection_budget() {
    let mut fixture = fixture("budget_code", None, 1).await;
    let holder = fixture.connected_account().await;
    let refused_account = fixture.new_account().await;

    let answer = fixture.pair_with_code(&refused_account).await;

    assert_eq!(refused(answer), Code::ResourceExhausted);
    fixture.delete(&refused_account).await;
    fixture.delete(&holder).await;
}

#[tokio::test]
async fn pair_with_qr_reports_a_failed_bot_build() {
    let mut fixture = fixture("build_qr", Some(REFUSED_UPSTREAM), 0).await;
    let account = fixture.new_account().await;

    let answer = fixture.pair_with_qr(&account).await;

    assert_eq!(refused(answer), Code::Unavailable);
    fixture.delete(&account).await;
}

/// Nothing is connecting after a failed build, and `GetAccountStatus` is what
/// the edge reads to know that.
#[tokio::test]
async fn a_failed_bot_build_leaves_the_account_disconnected() {
    let mut fixture = fixture("build_state", Some(REFUSED_UPSTREAM), 0).await;
    let account = fixture.new_account().await;

    let connect = pb::ConnectAccountRequest {
        account: account_of(&account),
        backfill_history: false,
    };
    let answer = fixture.accounts.connect_account(connect).await;
    answer.expect_err("the build must fail");

    assert_eq!(
        fixture.state_of(&account).await,
        pb::ConnectionState::Disconnected
    );
    fixture.delete(&account).await;
}

/// A terminal state comes from an event, and a connect that never started a bot
/// has no business erasing it.
#[tokio::test]
async fn a_failed_bot_build_keeps_a_logged_out_account_logged_out() {
    let mut fixture = fixture("build_terminal", Some(REFUSED_UPSTREAM), 0).await;
    let account = fixture.new_account().await;
    account
        .state_tx
        .send(pb::ConnectionState::LoggedOut)
        .expect("set the state");

    let connect = pb::ConnectAccountRequest {
        account: account_of(&account),
        backfill_history: false,
    };
    let answer = fixture.accounts.connect_account(connect).await;
    answer.expect_err("the build must fail");

    assert_eq!(
        fixture.state_of(&account).await,
        pb::ConnectionState::LoggedOut
    );
    fixture.delete(&account).await;
}

/// The budget counts bots, so a running account is past it: a second stream
/// joins the pairing in progress instead of being refused.
#[tokio::test]
async fn a_second_pair_with_qr_joins_the_pairing_in_progress() {
    let mut fixture = fixture("second_stream", None, 1).await;
    let account = fixture.new_account().await;
    let _first = fixture.pair_with_qr(&account).await.expect("first stream");
    let deadline = Instant::now() + WAIT;
    while !account.is_running().await {
        assert!(Instant::now() < deadline, "the connect never started");
        // not a sync point: on a build that connects in the background nothing announces the start, so it is polled with a deadline
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let mut second = fixture.pair_with_qr(&account).await.expect("second stream");
    account.publisher.publish_lib(Arc::new(Event::PairingQrCode(
        PairingQrCode::builder()
            .code(QR.to_string())
            .timeout(Duration::from_secs(60))
            .build(),
    )));

    let update = tokio::time::timeout(WAIT, second.next()).await;
    let update = update.expect("no QR within the wait");
    let update = update.expect("the stream ended before the QR");
    assert_eq!(
        update.expect("pairing stream").event,
        Some(pb::pairing_update::Event::QrCode(QR.to_string()))
    );
    fixture.delete(&account).await;
}
