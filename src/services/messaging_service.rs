//! MessagingService: text/reaction/edit/delete/presence/mark-read + media send,
//! chat actions, contact/poll/PTV sends, poll voting/tallying, and status
//! posting.

use std::sync::Arc;

use tonic::{Request, Response, Status, Streaming};

use super::{client_of, require_field, require_jid};
use crate::domain::jid_parse::parse_jid;
use crate::domain::messaging::{self, send_result_to_proto};
use crate::domain::{chat_actions, media_transfer, polls, send_rich, status};
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
        Ok(Response::new(send_result_to_proto(
            result.message_id,
            &result.to,
        )))
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
        Ok(Response::new(send_result_to_proto(
            result.message_id,
            &result.to,
        )))
    }

    async fn send_reaction(
        &self,
        request: Request<pb::SendReactionRequest>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let target = require_field(req.target, "target")?;
        let result = messaging::send_reaction(&client, &target, &req.emoji).await?;
        Ok(Response::new(send_result_to_proto(
            result.message_id,
            &result.to,
        )))
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

    async fn mark_unread(
        &self,
        request: Request<pb::MarkReadRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let chat = parse_jid(&require_jid(req.chat)?)?;
        chat_actions::mark_unread(&client, &chat).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn star_message(
        &self,
        request: Request<pb::StarMessageRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let target = require_field(req.target, "target")?;
        chat_actions::star_message(&client, &target, req.starred).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn archive_chat(
        &self,
        request: Request<pb::ArchiveChatRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let chat = parse_jid(&require_jid(req.chat)?)?;
        chat_actions::archive_chat(&client, chat, req.archived).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn pin_chat(
        &self,
        request: Request<pb::PinChatRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let chat = parse_jid(&require_jid(req.chat)?)?;
        chat_actions::pin_chat(&client, chat, req.pinned).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn mute_chat(
        &self,
        request: Request<pb::MuteChatRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let chat = parse_jid(&require_jid(req.chat)?)?;
        chat_actions::mute_chat(&client, chat, req.muted, req.mute_until_ms).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn delete_chat(
        &self,
        request: Request<pb::DeleteChatRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let chat = parse_jid(&require_jid(req.chat)?)?;
        chat_actions::delete_chat(&client, chat, req.delete_media).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn send_contact(
        &self,
        request: Request<pb::SendContactRequest>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let to = parse_jid(&require_jid(req.to.clone())?)?;
        let result = send_rich::send_contact(&client, to, &req).await?;
        Ok(Response::new(send_result_to_proto(
            result.message_id,
            &result.to,
        )))
    }

    async fn send_poll(
        &self,
        request: Request<pb::SendPollRequest>,
    ) -> Result<Response<pb::SendPollResult>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let to = parse_jid(&require_jid(req.to.clone())?)?;
        let (result, message_secret) = send_rich::send_poll(&client, to, &req).await?;
        // The poll key reuses send_result_to_proto's shape; the message_secret
        // rides alongside so the edge can decrypt incoming votes.
        Ok(Response::new(pb::SendPollResult {
            key: send_result_to_proto(result.message_id, &result.to).key,
            message_secret,
        }))
    }

    async fn send_poll_vote(
        &self,
        request: Request<pb::SendPollVoteRequest>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let chat = parse_jid(&require_jid(req.chat.clone())?)?;
        let result = polls::send_vote(&client, chat, &req).await?;
        Ok(Response::new(send_result_to_proto(
            result.message_id,
            &result.to,
        )))
    }

    // The one RPC on this service that sends nothing: it opens votes the edge
    // already holds. It lives here because it needs the account's identity
    // store, which is what the edge cannot reproduce (issue #13).
    async fn aggregate_poll_votes(
        &self,
        request: Request<pb::AggregatePollVotesRequest>,
    ) -> Result<Response<pb::PollTally>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        Ok(Response::new(polls::aggregate_votes(&client, &req).await?))
    }

    async fn post_status_text(
        &self,
        request: Request<pb::PostStatusTextRequest>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let result = status::post_status_text(&client, &req).await?;
        Ok(Response::new(send_result_to_proto(
            result.message_id,
            &result.to,
        )))
    }

    async fn post_status_media(
        &self,
        request: Request<Streaming<pb::PostStatusMediaChunk>>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let mut stream = request.into_inner();
        let first = stream
            .message()
            .await?
            .ok_or_else(|| Status::invalid_argument("empty status media stream"))?;
        let header = match first.part {
            Some(pb::post_status_media_chunk::Part::Header(h)) => h,
            _ => return Err(Status::invalid_argument("first chunk must be the header")),
        };
        let client = client_of(&self.registry, header.account.as_ref()).await?;
        // Same inline-only contract as SendMedia: the core fetches no URLs.
        let data = collect_status_media(&mut stream, self.media_max_bytes).await?;
        let result = status::post_status_media(&client, &header, data).await?;
        Ok(Response::new(send_result_to_proto(
            result.message_id,
            &result.to,
        )))
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

/// Gather inline status-media chunks (after the header) up to the byte limit.
/// A parallel of `collect_inline` for the distinct PostStatusMediaChunk oneof
/// (prost generates no shared trait over the two chunk types).
async fn collect_status_media(
    stream: &mut Streaming<pb::PostStatusMediaChunk>,
    max_bytes: u64,
) -> Result<Vec<u8>, Status> {
    let mut data = Vec::new();
    while let Some(chunk) = stream.message().await? {
        match chunk.part {
            Some(pb::post_status_media_chunk::Part::Chunk(bytes)) => {
                data.extend_from_slice(&bytes);
                if data.len() as u64 > max_bytes {
                    return Err(Status::resource_exhausted("media exceeds size limit"));
                }
            }
            Some(pb::post_status_media_chunk::Part::Header(_)) => {
                return Err(Status::invalid_argument("unexpected second header"));
            }
            None => {}
        }
    }
    Ok(data)
}
