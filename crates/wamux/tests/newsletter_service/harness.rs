//! One logged-in account behind a real socket, and the calls every
//! NewsletterService test shares.

use tonic::transport::Channel;
use wamux::proto::v1 as pb;
use wamux::proto::v1::newsletter_service_client::NewsletterServiceClient;
use wamux::stress::MockWaServer;

use crate::captured::{CHANNEL, VOTED_POLL};
use crate::common::mock_wire::account_ref;
use crate::common::{self, LoggedIn};

pub const NEWSLETTER: &str = "newsletter";
pub const MEX: &str = "w:mex";

/// The daemon serving a registry whose account is logged in against `mock`.
pub struct Fixture {
    pub mock: MockWaServer,
    pub logged: LoggedIn,
    pub channels: NewsletterServiceClient<Channel>,
    pub account: pb::AccountRef,
}

pub async fn fixture(test: &str) -> Fixture {
    let mock = MockWaServer::start().await.expect("start mock");
    let prefix = common::test_prefix("newsletter_service", test);
    let logged = common::logged_in_client(&mock, &prefix).await;
    let channel = common::serve_registry(logged.registry.clone()).await;
    let account = account_ref(&logged.handle.uuid.to_string());
    Fixture {
        mock,
        logged,
        channels: NewsletterServiceClient::new(channel),
        account,
    }
}

impl Fixture {
    pub async fn cleanup(self) {
        self.logged.cleanup().await;
    }

    pub fn jid_request(&self, jid: &str) -> pb::JidRequest {
        pb::JidRequest {
            account: Some(self.account.clone()),
            jid: Some(pb::Jid {
                value: jid.to_string(),
            }),
        }
    }

    pub fn history(&self, jid: &str, count: u32, before: u64) -> pb::GetNewsletterMessagesRequest {
        pb::GetNewsletterMessagesRequest {
            account: Some(self.account.clone()),
            jid: Some(pb::Jid {
                value: jid.to_string(),
            }),
            count,
            before,
        }
    }

    pub fn add_ons(&self, jid: &str, limit: u32) -> pb::GetMyNewsletterAddOnsRequest {
        pb::GetMyNewsletterAddOnsRequest {
            account: Some(self.account.clone()),
            jid: Some(pb::Jid {
                value: jid.to_string(),
            }),
            limit,
        }
    }

    pub fn vote(
        &self,
        jid: &str,
        server_id: u64,
        option_hashes: Vec<Vec<u8>>,
    ) -> pb::SendNewsletterPollVoteRequest {
        pb::SendNewsletterPollVoteRequest {
            account: Some(self.account.clone()),
            jid: Some(pb::Jid {
                value: jid.to_string(),
            }),
            server_id,
            option_hashes,
        }
    }
}

/// `sha256(option)`, the name a channel poll vote gives an option.
pub fn option_hash(option: &str) -> Vec<u8> {
    wacore::poll::compute_option_hash(option).to_vec()
}

/// Every one of the 6 RPCs, for `account`, with arguments valid in shape. The
/// name is the proto RPC, so a failure says which one.
pub async fn call_every_rpc(
    channels: &mut NewsletterServiceClient<Channel>,
    account: &pb::AccountRef,
) -> Vec<(&'static str, Result<(), tonic::Status>)> {
    let a = || Some(account.clone());
    let jid = || pb::JidRequest {
        account: a(),
        jid: Some(pb::Jid {
            value: CHANNEL.to_string(),
        }),
    };
    vec![
        (
            "ListSubscribedNewsletters",
            channels
                .list_subscribed_newsletters(account.clone())
                .await
                .map(drop),
        ),
        (
            "GetNewsletterMetadata",
            channels.get_newsletter_metadata(jid()).await.map(drop),
        ),
        (
            "GetNewsletterMessages",
            channels
                .get_newsletter_messages(pb::GetNewsletterMessagesRequest {
                    account: a(),
                    jid: Some(pb::Jid {
                        value: CHANNEL.to_string(),
                    }),
                    count: 5,
                    before: 0,
                })
                .await
                .map(drop),
        ),
        (
            "SendNewsletterPollVote",
            channels
                .send_newsletter_poll_vote(pb::SendNewsletterPollVoteRequest {
                    account: a(),
                    jid: Some(pb::Jid {
                        value: CHANNEL.to_string(),
                    }),
                    server_id: VOTED_POLL,
                    option_hashes: vec![option_hash("azul")],
                })
                .await
                .map(drop),
        ),
        (
            "GetMyNewsletterAddOns",
            channels
                .get_my_newsletter_add_ons(pb::GetMyNewsletterAddOnsRequest {
                    account: a(),
                    jid: Some(pb::Jid {
                        value: CHANNEL.to_string(),
                    }),
                    limit: 20,
                })
                .await
                .map(drop),
        ),
        (
            "SubscribeNewsletterLiveUpdates",
            channels
                .subscribe_newsletter_live_updates(jid())
                .await
                .map(drop),
        ),
    ]
}
