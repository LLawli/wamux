//! NewsletterService: read a channel's metadata, which is the only place its
//! name exists (issue #6), and a page of its history, which is the only place
//! the server's per-message tallies exist (issue #26). Its one write is the
//! poll vote, with the two reads that go with it: this account's own add-ons
//! and the live-tally subscription (#26 again). The library also offers
//! create/join/leave/update, but nothing needed those yet and the core does
//! not grow surface ahead of a caller.

use std::sync::Arc;

use tonic::{Request, Response, Status};

use super::client_of;
use crate::domain::newsletters;
use crate::proto::v1 as pb;
use crate::proto::v1::newsletter_service_server::NewsletterService;
use crate::state::AccountRegistry;

pub struct NewsletterSvc {
    registry: Arc<AccountRegistry>,
}

impl NewsletterSvc {
    pub fn new(registry: Arc<AccountRegistry>) -> Self {
        Self { registry }
    }
}

#[tonic::async_trait]
impl NewsletterService for NewsletterSvc {
    async fn list_subscribed_newsletters(
        &self,
        request: Request<pb::AccountRef>,
    ) -> Result<Response<pb::NewsletterList>, Status> {
        let account = request.into_inner();
        let client = client_of(&self.registry, Some(&account)).await?;
        Ok(Response::new(newsletters::list_subscribed(&client).await?))
    }

    async fn get_newsletter_metadata(
        &self,
        request: Request<pb::JidRequest>,
    ) -> Result<Response<pb::Newsletter>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        Ok(Response::new(
            newsletters::get_metadata(&client, &req.jid).await?,
        ))
    }

    async fn get_newsletter_messages(
        &self,
        request: Request<pb::GetNewsletterMessagesRequest>,
    ) -> Result<Response<pb::NewsletterMessageList>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        Ok(Response::new(
            newsletters::get_messages(&client, &req).await?,
        ))
    }

    async fn send_newsletter_poll_vote(
        &self,
        request: Request<pb::SendNewsletterPollVoteRequest>,
    ) -> Result<Response<pb::SendNewsletterPollVoteResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        Ok(Response::new(
            newsletters::send_poll_vote(&client, &req).await?,
        ))
    }

    async fn get_my_newsletter_add_ons(
        &self,
        request: Request<pb::GetMyNewsletterAddOnsRequest>,
    ) -> Result<Response<pb::NewsletterMyAddOnsList>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        Ok(Response::new(
            newsletters::get_my_addons(&client, &req).await?,
        ))
    }

    async fn subscribe_newsletter_live_updates(
        &self,
        request: Request<pb::JidRequest>,
    ) -> Result<Response<pb::SubscribeNewsletterLiveUpdatesResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        Ok(Response::new(
            newsletters::subscribe_live_updates(&client, &req.jid).await?,
        ))
    }
}
