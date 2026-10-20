//! CallService: call control that needs no media stack (#200).

use std::sync::Arc;

use tonic::{Request, Response, Status};
use wamux_types::Jid;

use super::client_of;
use crate::domain::calls;
use crate::proto::v1 as pb;
use crate::proto::v1::call_service_server::CallService;
use crate::state::AccountRegistry;

pub struct CallSvc {
    registry: Arc<AccountRegistry>,
}

impl CallSvc {
    pub fn new(registry: Arc<AccountRegistry>) -> Self {
        Self { registry }
    }
}

#[tonic::async_trait]
impl CallService for CallSvc {
    async fn reject_call(
        &self,
        request: Request<pb::RejectCallRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let peer = Jid::from_required_wire(req.peer)?;
        let call_creator = Jid::from_required_wire(req.call_creator)?;
        calls::reject_call(&client, &req.call_id, &peer, &call_creator).await?;
        Ok(Response::new(pb::Empty {}))
    }
}
