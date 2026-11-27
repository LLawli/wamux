//! #274: `ListAccounts` carries each account's own jid, the one
//! `GetAccountStatus` answers: the paired device's phone number (or LID) from
//! the store, unset while the account has never paired. The listing used to
//! leave it unset for every account. Honors `WAMUX_TEST_ENGINE`.

use crate::common;

use tonic::transport::Channel;
use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::stress::MockWaServer;

use common::mock_wire::account_ref;

struct Fixture {
    _mock: MockWaServer,
    logged: common::LoggedIn,
    accounts: AccountServiceClient<Channel>,
    prefix: String,
}

async fn fixture(test: &str) -> Fixture {
    let mock = MockWaServer::start().await.expect("start mock");
    let prefix = common::test_prefix("account_listing", test);
    let logged = common::logged_in_client(&mock, &prefix).await;
    let channel = common::serve_registry(logged.registry.clone()).await;
    Fixture {
        _mock: mock,
        logged,
        accounts: AccountServiceClient::new(channel),
        prefix,
    }
}

impl Fixture {
    fn logged_uuid(&self) -> String {
        self.logged.handle.uuid.to_string()
    }

    /// The listed account with `uuid`; every test owns its accounts, so the
    /// lookup is by uuid rather than by position.
    async fn listed(&mut self, uuid: &str) -> pb::Account {
        let listing = self
            .accounts
            .list_accounts(pb::ListAccountsRequest {})
            .await
            .expect("ListAccounts")
            .into_inner();
        listing
            .accounts
            .into_iter()
            .find(|account| account.uuid == uuid)
            .unwrap_or_else(|| panic!("account {uuid} is not listed"))
    }

    async fn status_jid(&mut self, uuid: &str) -> Option<pb::Jid> {
        self.accounts
            .get_account_status(account_ref(uuid))
            .await
            .expect("GetAccountStatus")
            .into_inner()
            .jid
    }

    async fn create(&mut self, suffix: &str) -> pb::Account {
        let request = pb::CreateAccountRequest {
            external_ref: Some(format!("{}-{suffix}", self.prefix)),
        };
        self.accounts
            .create_account(request)
            .await
            .expect("CreateAccount")
            .into_inner()
    }

    async fn delete(&mut self, uuid: &str) {
        self.accounts
            .delete_account(account_ref(uuid))
            .await
            .expect("DeleteAccount");
    }
}

#[tokio::test]
async fn list_accounts_carries_a_paired_accounts_jid() {
    let mut f = fixture("paired").await;
    let uuid = f.logged_uuid();
    let status = f
        .status_jid(&uuid)
        .await
        .expect("a paired account has a jid");
    assert!(!status.value.is_empty(), "{status:?}");
    let listed = f.listed(&uuid).await;
    assert_eq!(
        listed.jid,
        Some(status),
        "the listing says what the status says"
    );
    f.logged.cleanup().await;
}

#[tokio::test]
async fn list_accounts_leaves_an_unpaired_accounts_jid_unset() {
    let mut f = fixture("unpaired").await;
    let created = f.create("never-paired").await;
    assert_eq!(f.status_jid(&created.uuid).await, None);
    assert_eq!(f.listed(&created.uuid).await.jid, None);
    let paired = f.logged_uuid();
    assert!(
        f.listed(&paired).await.jid.is_some(),
        "the paired account next to it keeps its jid"
    );
    f.delete(&created.uuid).await;
    f.logged.cleanup().await;
}

/// The jid lives in the store, not in the running client: a stopped account
/// still has it, as `GetAccountStatus` already answers.
#[tokio::test]
async fn list_accounts_keeps_a_stopped_accounts_jid() {
    let mut f = fixture("stopped").await;
    let uuid = f.logged_uuid();
    let before = f
        .status_jid(&uuid)
        .await
        .expect("a paired account has a jid");
    f.accounts
        .disconnect_account(account_ref(&uuid))
        .await
        .expect("DisconnectAccount");
    let listed = f.listed(&uuid).await;
    assert_eq!(listed.state(), pb::ConnectionState::Disconnected);
    assert_eq!(listed.jid, Some(before));
    f.logged.cleanup().await;
}

/// A new account has never paired, so it has no jid to answer.
#[tokio::test]
async fn create_account_answers_without_a_jid() {
    let mut f = fixture("created").await;
    let created = f.create("fresh").await;
    assert!(!created.uuid.is_empty());
    assert_eq!(created.jid, None);
    f.delete(&created.uuid).await;
    f.logged.cleanup().await;
}
