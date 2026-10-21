//! Logout through the socket (#205): the `LoggedOut` the library dispatches on
//! a `Logout` RPC reaches the subscriber, and the account ends LOGGED_OUT.
//!
//! The library dispatches `LoggedOut` and shuts the client down in the same
//! call, so an event delivery that drops events or cancels in-flight callbacks
//! at shutdown loses exactly this one: the edge sees no logout, and the
//! account reads DISCONNECTED instead. The account is logged in against
//! `MockWaServer`, which answers the unlink IQ with a bare result. Honors
//! `WAMUX_TEST_ENGINE`; `scripts/ci.sh` runs it on both engines.
#![cfg(feature = "stress")]

use std::time::Duration;

use tokio_stream::StreamExt;
use tonic::transport::Channel;
use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::stress::MockWaServer;

// Only a subset of the shared helpers is used per test binary.
#[allow(dead_code)]
mod common;

use common::mock_wire::account_ref;

/// Long enough for a loaded CI runner, short enough to fail fast when the
/// event never comes.
const EVENT_WAIT: Duration = Duration::from_secs(10);

struct Fixture {
    _mock: MockWaServer,
    logged: common::LoggedIn,
    accounts: AccountServiceClient<Channel>,
    events: EventServiceClient<Channel>,
    account: pb::AccountRef,
}

async fn fixture(test: &str) -> Fixture {
    let mock = MockWaServer::start().await.expect("start mock");
    let prefix = common::test_prefix("logout_events", test);
    let logged = common::logged_in_client(&mock, &prefix).await;
    let channel = common::serve_registry(logged.registry.clone()).await;
    let account = account_ref(&logged.handle.uuid.to_string());
    Fixture {
        _mock: mock,
        logged,
        accounts: AccountServiceClient::new(channel.clone()),
        events: EventServiceClient::new(channel),
        account,
    }
}

/// The first ConnectionStateChanged on the stream whose state is `wanted`.
async fn connection_event(
    stream: &mut tonic::Streaming<pb::EventEnvelope>,
    wanted: pb::ConnectionState,
) -> pb::ConnectionStateChanged {
    let wait = async {
        while let Some(envelope) = stream.next().await {
            let envelope = envelope.expect("event stream");
            if let Some(pb::event_envelope::Event::Connection(change)) = envelope.event
                && change.state == wanted as i32
            {
                return change;
            }
        }
        panic!("the event stream ended before a {wanted:?} ConnectionStateChanged");
    };
    tokio::time::timeout(EVENT_WAIT, wait)
        .await
        .unwrap_or_else(|_| panic!("no {wanted:?} ConnectionStateChanged within {EVENT_WAIT:?}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn logout_relays_logged_out_and_leaves_the_account_logged_out() {
    let mut f = fixture("logout_relays_logged_out_and_leaves_the_account_logged_out").await;
    let subscribe = pb::SubscribeRequest {
        selector: Some(pb::subscribe_request::Selector::Account(f.account.clone())),
        replay_from_ring: 0,
    };
    let mut stream = f
        .events
        .subscribe_events(subscribe)
        .await
        .expect("subscribe")
        .into_inner();
    f.accounts.logout(f.account.clone()).await.expect("logout");

    let change = connection_event(&mut stream, pb::ConnectionState::LoggedOut).await;
    let info = change
        .logged_out
        .expect("a LOGGED_OUT change carries its info");
    assert_eq!(info.reason, pb::LogoutReason::LoggedOut as i32);
    assert!(!info.on_connect);

    // The run loop exits right after the logout. Its supervisor must keep the
    // LOGGED_OUT the event set, not overwrite it with DISCONNECTED.
    let accounts = f.accounts.clone();
    let account = f.account.clone();
    common::poll_until("the account to read LOGGED_OUT", EVENT_WAIT, || {
        let mut accounts = accounts.clone();
        let account = account.clone();
        async move {
            let status = accounts
                .get_account_status(account)
                .await
                .ok()?
                .into_inner();
            (status.state == pb::ConnectionState::LoggedOut as i32).then_some(())
        }
    })
    .await;
    f.logged.cleanup().await;
}
