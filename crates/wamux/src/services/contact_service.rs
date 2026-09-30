//! ContactService: number checks, profile pictures, push name/about, presence
//! subscription.

use std::sync::Arc;

use tonic::{Request, Response, Status};

use super::client_of;
use crate::domain::{contacts, lid_mapping};
use crate::proto::v1 as pb;
use crate::proto::v1::contact_service_server::ContactService;
use crate::state::AccountRegistry;

pub struct ContactSvc {
    registry: Arc<AccountRegistry>,
}

impl ContactSvc {
    pub fn new(registry: Arc<AccountRegistry>) -> Self {
        Self { registry }
    }
}

#[tonic::async_trait]
impl ContactService for ContactSvc {
    async fn check_on_whats_app(
        &self,
        request: Request<pb::CheckOnWhatsAppRequest>,
    ) -> Result<Response<pb::CheckOnWhatsAppResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let results = contacts::check_on_whatsapp(client, &req.jids).await?;
        Ok(Response::new(pb::CheckOnWhatsAppResponse { results }))
    }

    async fn get_profile_picture(
        &self,
        request: Request<pb::JidRequest>,
    ) -> Result<Response<pb::ProfilePictureResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        Ok(Response::new(
            contacts::get_profile_picture(&client, &req.jid).await?,
        ))
    }

    async fn set_profile_picture(
        &self,
        request: Request<pb::SetProfilePictureRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        contacts::set_profile_picture(&client, req.image).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn remove_profile_picture(
        &self,
        request: Request<pb::AccountRef>,
    ) -> Result<Response<pb::Empty>, Status> {
        let reference = request.into_inner();
        let client = client_of(&self.registry, Some(&reference)).await?;
        contacts::remove_profile_picture(&client).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn get_push_name(
        &self,
        request: Request<pb::AccountRef>,
    ) -> Result<Response<pb::PushNameResponse>, Status> {
        let reference = request.into_inner();
        let client = client_of(&self.registry, Some(&reference)).await?;
        Ok(Response::new(contacts::get_push_name(&client).await?))
    }

    async fn set_push_name(
        &self,
        request: Request<pb::SetPushNameRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        contacts::set_push_name(&client, &req.push_name).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn get_about(
        &self,
        request: Request<pb::JidRequest>,
    ) -> Result<Response<pb::AboutResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        Ok(Response::new(contacts::get_about(client, &req.jid).await?))
    }

    async fn get_business_profile(
        &self,
        request: Request<pb::JidRequest>,
    ) -> Result<Response<pb::BusinessProfileResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        Ok(Response::new(
            contacts::get_business_profile(&client, &req.jid).await?,
        ))
    }

    async fn subscribe_presence(
        &self,
        request: Request<pb::SubscribePresenceRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        contacts::subscribe_presence(&client, &req.jid).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn resolve_lid_pn(
        &self,
        request: Request<pb::ResolveLidPnRequest>,
    ) -> Result<Response<pb::ResolveLidPnResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let results = lid_mapping::resolve_lid_pn(&client, &req.jids).await?;
        Ok(Response::new(pb::ResolveLidPnResponse { results }))
    }

    /// Storage-side, so it needs a known account but not a connected one: an
    /// edge reconciling its own store after a restart has nothing live yet.
    async fn list_lid_mappings(
        &self,
        request: Request<pb::AccountRef>,
    ) -> Result<Response<pb::ListLidMappingsResponse>, Status> {
        let reference = request.into_inner();
        let handle = self.registry.resolve(Some(&reference))?;
        let backend = self.registry.storage().device_backend(handle.device_id);
        let mappings = lid_mapping::list_lid_mappings(backend).await?;
        Ok(Response::new(pb::ListLidMappingsResponse { mappings }))
    }
}
