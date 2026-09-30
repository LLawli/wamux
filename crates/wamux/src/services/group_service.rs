//! GroupService: create/manage groups, participants, metadata, invite links.

use std::sync::Arc;

use tonic::{Request, Response, Status};

use super::client_of;
use crate::domain::groups;
use crate::proto::v1 as pb;
use crate::proto::v1::group_service_server::GroupService;
use crate::state::AccountRegistry;

pub struct GroupSvc {
    registry: Arc<AccountRegistry>,
}

impl GroupSvc {
    pub fn new(registry: Arc<AccountRegistry>) -> Self {
        Self { registry }
    }
}

#[tonic::async_trait]
impl GroupService for GroupSvc {
    async fn create_group(
        &self,
        request: Request<pb::CreateGroupRequest>,
    ) -> Result<Response<pb::GroupJidResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        Ok(Response::new(
            groups::create_group(&client, &req.subject, &req.participants).await?,
        ))
    }

    async fn add_participants(
        &self,
        request: Request<pb::ParticipantsRequest>,
    ) -> Result<Response<pb::ParticipantsResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let results = groups::add_participants(&client, &req.group_jid, &req.participants).await?;
        Ok(Response::new(results))
    }

    async fn remove_participants(
        &self,
        request: Request<pb::ParticipantsRequest>,
    ) -> Result<Response<pb::ParticipantsResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let results =
            groups::remove_participants(&client, &req.group_jid, &req.participants).await?;
        Ok(Response::new(results))
    }

    async fn promote_admins(
        &self,
        request: Request<pb::ParticipantsRequest>,
    ) -> Result<Response<pb::ParticipantsResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let results = groups::promote(&client, &req.group_jid, &req.participants).await?;
        Ok(Response::new(results))
    }

    async fn demote_admins(
        &self,
        request: Request<pb::ParticipantsRequest>,
    ) -> Result<Response<pb::ParticipantsResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let results = groups::demote(&client, &req.group_jid, &req.participants).await?;
        Ok(Response::new(results))
    }

    async fn set_group_subject(
        &self,
        request: Request<pb::GroupTextRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        groups::set_subject(&client, &req.group_jid, &req.text).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn set_group_description(
        &self,
        request: Request<pb::GroupTextRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        groups::set_description(&client, &req.group_jid, &req.text).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn get_group_metadata(
        &self,
        request: Request<pb::GroupRef>,
    ) -> Result<Response<pb::GroupMetadataResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        Ok(Response::new(
            groups::get_metadata(&client, &req.group_jid).await?,
        ))
    }

    async fn get_invite_link(
        &self,
        request: Request<pb::GroupRef>,
    ) -> Result<Response<pb::InviteLinkResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        Ok(Response::new(
            groups::invite_link(&client, &req.group_jid, false).await?,
        ))
    }

    async fn revoke_invite_link(
        &self,
        request: Request<pb::GroupRef>,
    ) -> Result<Response<pb::InviteLinkResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        Ok(Response::new(
            groups::invite_link(&client, &req.group_jid, true).await?,
        ))
    }

    async fn join_with_invite(
        &self,
        request: Request<pb::JoinWithInviteRequest>,
    ) -> Result<Response<pb::GroupJidResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        Ok(Response::new(
            groups::join_with_invite(&client, &req.code).await?,
        ))
    }

    async fn list_participating(
        &self,
        request: Request<pb::AccountRef>,
    ) -> Result<Response<pb::ListGroupsResponse>, Status> {
        let reference = request.into_inner();
        let client = client_of(&self.registry, Some(&reference)).await?;
        let groups = groups::list_participating(client).await?;
        Ok(Response::new(pb::ListGroupsResponse { groups }))
    }

    async fn leave_group(
        &self,
        request: Request<pb::GroupRef>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        groups::leave(&client, &req.group_jid).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn set_group_announce(
        &self,
        request: Request<pb::GroupToggleRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        groups::set_announce(&client, &req.group_jid, req.enabled).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn set_group_locked(
        &self,
        request: Request<pb::GroupToggleRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        groups::set_locked(&client, &req.group_jid, req.enabled).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn set_group_ephemeral(
        &self,
        request: Request<pb::GroupEphemeralRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        groups::set_ephemeral(&client, &req.group_jid, req.expiration_seconds).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn preview_invite(
        &self,
        request: Request<pb::PreviewInviteRequest>,
    ) -> Result<Response<pb::GroupMetadataResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        Ok(Response::new(
            groups::preview_invite(&client, &req.code).await?,
        ))
    }

    async fn set_group_photo(
        &self,
        request: Request<pb::SetGroupPhotoRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        groups::set_photo(&client, &req.group_jid, req.image).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn get_membership_requests(
        &self,
        request: Request<pb::GroupRef>,
    ) -> Result<Response<pb::MembershipRequestsResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        Ok(Response::new(
            groups::membership_requests(&client, &req.group_jid).await?,
        ))
    }

    async fn approve_membership_requests(
        &self,
        request: Request<pb::ParticipantsRequest>,
    ) -> Result<Response<pb::ParticipantsResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let results =
            groups::approve_membership(&client, &req.group_jid, &req.participants).await?;
        Ok(Response::new(results))
    }

    async fn reject_membership_requests(
        &self,
        request: Request<pb::ParticipantsRequest>,
    ) -> Result<Response<pb::ParticipantsResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let results = groups::reject_membership(&client, &req.group_jid, &req.participants).await?;
        Ok(Response::new(results))
    }
}
