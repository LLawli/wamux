//! MessagingService: text/reaction/edit/delete/presence/mark-read + media send.

use std::sync::Arc;

use tonic::{Request, Response, Status, Streaming};

use super::{client_of, require_field, require_jid};
use crate::domain::jid_parse::parse_jid;
use crate::domain::media_transfer;
use crate::domain::messaging::{self, send_result_to_proto};
use crate::proto::v1 as pb;
use crate::proto::v1::messaging_service_server::MessagingService;
use crate::state::AccountRegistry;

pub struct MessagingSvc {
    registry: Arc<AccountRegistry>,
    media_max_bytes: u64,
}

impl MessagingSvc {
    pub fn new(registry: Arc<AccountRegistry>, media_max_bytes: u64) -> Self {
        Self {
            registry,
            media_max_bytes,
        }
    }
}

#[tonic::async_trait]
impl MessagingService for MessagingSvc {
    async fn send_text(
        &self,
        request: Request<pb::SendTextRequest>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        // Routing resolved here; the whole request passes wire-shaped to the
        // domain (same pattern as send_media's header).
        let to = parse_jid(&require_jid(req.to.clone())?)?;
        let result = messaging::send_text(&client, to, &req).await?;
        Ok(Response::new(send_result_to_proto(result)))
    }

    async fn send_media(
        &self,
        request: Request<Streaming<pb::SendMediaChunk>>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let mut stream = request.into_inner();
        let first = stream
            .message()
            .await?
            .ok_or_else(|| Status::invalid_argument("empty media stream"))?;
        let header = match first.part {
            Some(pb::send_media_chunk::Part::Header(h)) => h,
            _ => return Err(Status::invalid_argument("first chunk must be the header")),
        };

        let client = client_of(&self.registry, header.account.as_ref()).await?;
        let to = parse_jid(&require_jid(header.to.clone())?)?;

        // Media always streams inline; the core never fetches URLs (that fetch
        // policy + SSRF surface is the edge's job).
        let data = collect_inline(&mut stream, self.media_max_bytes).await?;

        let result = media_transfer::send_media(&client, to, &header, data).await?;
        Ok(Response::new(send_result_to_proto(result)))
    }

    async fn send_reaction(
        &self,
        request: Request<pb::SendReactionRequest>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let target = require_field(req.target, "target")?;
        let result = messaging::send_reaction(&client, &target, &req.emoji).await?;
        Ok(Response::new(send_result_to_proto(result)))
    }

    async fn edit_message(
        &self,
        request: Request<pb::EditMessageRequest>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let target = require_field(req.target, "target")?;
        let new_id = messaging::edit_message(&client, &target, &req.new_text).await?;
        Ok(Response::new(pb::SendResult {
            key: Some(pb::MessageKey {
                remote_jid: target.remote_jid,
                id: new_id,
                from_me: true,
                participant: String::new(),
            }),
            server_timestamp: 0,
        }))
    }

    async fn delete_message(
        &self,
        request: Request<pb::DeleteMessageRequest>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let target = require_field(req.target, "target")?;
        messaging::delete_message(&client, &target, req.for_everyone).await?;
        Ok(Response::new(pb::SendResult {
            key: Some(target),
            server_timestamp: 0,
        }))
    }

    async fn fetch_message_history(
        &self,
        request: Request<pb::FetchMessageHistoryRequest>,
    ) -> Result<Response<pb::FetchMessageHistoryResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let chat = parse_jid(&require_jid(req.chat)?)?;
        let session_id = messaging::fetch_message_history(
            &client,
            chat,
            &req.oldest_msg_id,
            req.oldest_msg_from_me,
            req.oldest_msg_timestamp_ms,
            req.count,
        )
        .await?;
        Ok(Response::new(pb::FetchMessageHistoryResponse {
            session_id,
        }))
    }

    async fn send_presence(
        &self,
        request: Request<pb::SendPresenceRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let chat = parse_jid(&require_jid(req.chat)?)?;
        messaging::send_presence(&client, chat, &req.state).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn mark_read(
        &self,
        request: Request<pb::MarkReadRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let chat = parse_jid(&require_jid(req.chat)?)?;
        messaging::mark_read(&client, &chat).await?;
        Ok(Response::new(pb::Empty {}))
    }
}

/// Gather inline media chunks (after the header) up to the byte limit.
async fn collect_inline(
    stream: &mut Streaming<pb::SendMediaChunk>,
    max_bytes: u64,
) -> Result<Vec<u8>, Status> {
    let mut data = Vec::new();
    while let Some(chunk) = stream.message().await? {
        match chunk.part {
            Some(pb::send_media_chunk::Part::Chunk(bytes)) => {
                data.extend_from_slice(&bytes);
                if data.len() as u64 > max_bytes {
                    return Err(Status::resource_exhausted("media exceeds size limit"));
                }
            }
            Some(pb::send_media_chunk::Part::Header(_)) => {
                return Err(Status::invalid_argument("unexpected second header"));
            }
            None => {}
        }
    }
    Ok(data)
}
