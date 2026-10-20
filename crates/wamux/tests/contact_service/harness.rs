//! One logged-in account behind a real socket, and the wire helpers the
//! ContactService tests share.

use tonic::transport::Channel;
use wacore::store::traits::LidPnMappingEntry;
use wacore_binary::Node;
use wamux::proto::v1 as pb;
use wamux::proto::v1::contact_service_client::ContactServiceClient;
use wamux::stress::MockWaServer;

use crate::common::mock_wire::{account_ref, operation};
use crate::common::{self, LoggedIn};

pub const BLOCKLIST: &str = "blocklist";
/// A contact's two sides, as the client learned them.
pub const CONTACT_LID: &str = "169815004184633@lid";
pub const CONTACT_PN: &str = "5511999000111@s.whatsapp.net";
/// A contact the client has no LID<->PN pair for.
pub const STRANGER_PN: &str = "5511988000222@s.whatsapp.net";

/// The daemon serving a registry whose account is logged in against `mock`.
pub struct Fixture {
    pub mock: MockWaServer,
    pub logged: LoggedIn,
    pub contacts: ContactServiceClient<Channel>,
    pub account: pb::AccountRef,
}

pub async fn fixture(test: &str) -> Fixture {
    let mock = MockWaServer::start().await.expect("start mock");
    let prefix = common::test_prefix("contact_service", test);
    let logged = common::logged_in_client(&mock, &prefix).await;
    let channel = common::serve_registry(logged.registry.clone()).await;
    let account = account_ref(&logged.handle.uuid.to_string());
    Fixture {
        mock,
        logged,
        contacts: ContactServiceClient::new(channel),
        account,
    }
}

impl Fixture {
    pub async fn cleanup(self) {
        self.logged.cleanup().await;
    }

    pub fn jid_request(&self, value: &str) -> pb::JidRequest {
        pb::JidRequest {
            account: Some(self.account.clone()),
            jid: jid(value),
        }
    }

    /// Persist `CONTACT_LID` <-> `CONTACT_PN` in the store the library reads on
    /// a cache miss, as if the client had learned the pair from a usync.
    pub async fn seed_contact_mapping(&self) {
        self.logged
            .registry
            .storage()
            .device_backend(self.logged.handle.device_id)
            .put_lid_mapping(&LidPnMappingEntry {
                lid: "169815004184633".to_string(),
                phone_number: "5511999000111".to_string(),
                created_at: 1_717_932_000,
                updated_at: 1_717_932_000,
                learning_source: "usync".to_string(),
            })
            .await
            .expect("seed lid mapping");
    }

    /// Every `<iq xmlns="blocklist">` the client sent so far.
    pub fn blocklist_iqs(&self) -> Vec<Node> {
        self.mock
            .client_iqs()
            .into_iter()
            .filter(|iq| attr_of(iq, "xmlns").as_deref() == Some(BLOCKLIST))
            .collect()
    }
}

/// A jid field as the edge fills it.
pub fn jid(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

fn attr_of(node: &Node, key: &str) -> Option<String> {
    crate::common::mock_wire::attr(node, key)
}

/// The `<item>` a blocklist update carries.
pub fn blocklist_item(iq: &Node) -> &Node {
    let item = operation(iq).expect("a blocklist update carries an <item>");
    assert_eq!(item.tag.as_ref(), "item", "operation tag");
    item
}
