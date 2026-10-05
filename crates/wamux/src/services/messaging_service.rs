//! MessagingService: text/reaction/edit/delete/presence/mark-read + media send,
//! chat actions, contact/poll/PTV sends, poll voting/tallying, and status
//! posting.

use std::sync::Arc;

use tonic::{Request, Response, Status, Streaming};
use wamux_types::{
    ContactCard, InteractiveReply, Jid, MessageId, MessageTarget, NewPoll, OutgoingMedia,
    OutgoingText, PollVoteCast, PollVotesToTally, StatusMedia, StatusRevoke, StatusText,
};
use whatsapp_rust::SendResult;

use super::media_stream::MediaChunks;
use super::{account_of, client_of, missing_field, own_jid, require_typed_jid};
use crate::domain::messaging::{
    self, recipient_fanout_to_proto, send_result_to_proto, sent_message_key,
};
use crate::domain::{chat_actions, interactive_reply, media_transfer, polls, send_rich, status};
use crate::proto::v1 as pb;
use crate::proto::v1::messaging_service_server::MessagingService;
use crate::state::AccountRegistry;
use crate::state::send_echo::publish_sent;

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

impl MessagingSvc {
    /// Put a message this relay just sent on the event bus (issue #22).
    ///
    /// Every send reaches here, the ones the library builds itself (poll, vote,
    /// edit, revoke, status) included: since upstream #1406 `SendResult`
    /// carries the message the send encoded, so there is always a real payload
    /// to echo and never a reconstruction (#38).
    async fn echo(
        &self,
        handle: &Arc<crate::state::AccountHandle>,
        client: &whatsapp_rust::Client,
        result: &SendResult,
    ) {
        // An echo without an id cannot be deduplicated by a consumer, so it is
        // worse than no echo; the send itself already happened (#115).
        let Ok(message_id) = MessageId::new(result.message_id.clone()) else {
            tracing::warn!(chat = %result.to, "library returned an empty message id; echo skipped");
            return;
        };
        publish_sent(
            handle,
            &message_id,
            &Jid::from(result.to.clone()),
            own_jid(client).as_ref(),
            &result.message,
            self.registry.replay_max_event_bytes(),
        )
        .await;
    }

    /// The target of a DeleteMessage, with the account it acts on. A revoke for
    /// everyone is checked for shape and for a status BEFORE the account is
    /// looked up (#41); a delete-for-me looks the account up first, so an
    /// unknown account still answers NotFound for a malformed key (#101).
    async fn delete_target(
        &self,
        account: Option<&pb::AccountRef>,
        key: pb::MessageKey,
        for_everyone: bool,
    ) -> Result<
        (
            Arc<crate::state::AccountHandle>,
            Arc<whatsapp_rust::Client>,
            MessageTarget,
        ),
        Status,
    > {
        if !for_everyone {
            let (handle, client) = account_of(&self.registry, account).await?;
            return Ok((handle, client, MessageTarget::try_from(key)?));
        }
        let target = MessageTarget::try_from(key)?;
        messaging::refuse_status_revoke(&target, true)?;
        let (handle, client) = account_of(&self.registry, account).await?;
        Ok((handle, client, target))
    }

    /// Echo the send, then answer with its key. The shape of every send RPC
    /// whose response is the plain `SendResult`.
    async fn echoed(
        &self,
        handle: &Arc<crate::state::AccountHandle>,
        client: &whatsapp_rust::Client,
        result: SendResult,
    ) -> Response<pb::SendResult> {
        self.echo(handle, client, &result).await;
        Response::new(send_result_to_proto(
            result.message_id,
            &result.to,
            result.recipient_fanout,
        ))
    }
}

#[tonic::async_trait]
impl MessagingService for MessagingSvc {
    async fn send_text(
        &self,
        request: Request<pb::SendTextRequest>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let req = request.into_inner();
        let (handle, client) = account_of(&self.registry, req.account.as_ref()).await?;
        // Routing resolved here; the content converts once, after it (#115).
        let to = require_typed_jid(req.to.clone())?;
        let text = OutgoingText::try_from(req)?;
        let result = messaging::send_text(&client, to, &text).await?;
        Ok(self.echoed(&handle, &client, result).await)
    }

    async fn send_media(
        &self,
        request: Request<Streaming<pb::SendMediaChunk>>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let mut chunks = MediaChunks::Send(request.into_inner());
        let header = chunks
            .read_header("empty media stream")
            .await?
            .into_send()?;

        let (handle, client) = account_of(&self.registry, header.account.as_ref()).await?;
        let to = require_typed_jid(header.to.clone())?;

        // Media always streams inline; the core never fetches URLs (that fetch
        // policy + SSRF surface is the edge's job).
        let data = chunks.collect_bytes(self.media_max_bytes).await?;

        // Converted after the bytes: the kind was read in the domain, after
        // the stream, and the order of the errors is part of the contract.
        let media = OutgoingMedia::try_from(header)?;
        let result = media_transfer::send_media(&client, to, &media, data).await?;
        Ok(self.echoed(&handle, &client, result).await)
    }

    async fn send_reaction(
        &self,
        request: Request<pb::SendReactionRequest>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let req = request.into_inner();
        let (handle, client) = account_of(&self.registry, req.account.as_ref()).await?;
        let target = MessageTarget::try_from(req.target.ok_or_else(|| missing_field("target"))?)?;
        let result = messaging::send_reaction(&client, &target, &req.emoji).await?;
        Ok(self.echoed(&handle, &client, result).await)
    }

    async fn edit_message(
        &self,
        request: Request<pb::EditMessageRequest>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let req = request.into_inner();
        let (handle, client) = account_of(&self.registry, req.account.as_ref()).await?;
        let raw_key = req.target.ok_or_else(|| missing_field("target"))?;
        let target = MessageTarget::try_from(raw_key.clone())?;
        let result = messaging::edit_message(&client, &target, &req.new_text).await?;
        self.echo(&handle, &client, &result).await;
        // The response keeps naming the chat verbatim as the caller wrote it,
        // as before #38; the echo names the chat the library addressed.
        Ok(Response::new(pb::SendResult {
            key: Some(pb::MessageKey {
                remote_jid: raw_key.remote_jid,
                id: result.message_id,
                from_me: true,
                participant: String::new(),
            }),
            server_timestamp: 0,
            recipient_fanout: result.recipient_fanout.map(recipient_fanout_to_proto),
        }))
    }

    async fn delete_message(
        &self,
        request: Request<pb::DeleteMessageRequest>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let req = request.into_inner();
        let raw_key = req.target.ok_or_else(|| missing_field("target"))?;
        let (handle, client, target) = self
            .delete_target(req.account.as_ref(), raw_key.clone(), req.for_everyone)
            .await?;
        let revoke = messaging::delete_message(&client, &target, req.for_everyone).await?;
        if let Some(result) = &revoke {
            self.echo(&handle, &client, result).await;
        }
        // A delete-for-me sends nothing, so it has no fan-out to report.
        Ok(Response::new(pb::SendResult {
            key: Some(raw_key),
            server_timestamp: 0,
            recipient_fanout: revoke
                .and_then(|result| result.recipient_fanout)
                .map(recipient_fanout_to_proto),
        }))
    }

    async fn fetch_message_history(
        &self,
        request: Request<pb::FetchMessageHistoryRequest>,
    ) -> Result<Response<pb::FetchMessageHistoryResponse>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let chat = require_typed_jid(req.chat)?;
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
        let chat = require_typed_jid(req.chat)?;
        messaging::send_presence(&client, chat, &req.state).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn mark_read(
        &self,
        request: Request<pb::MarkReadRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let chat = require_typed_jid(req.chat)?;
        // Both of these were dropped on the floor before issue #20. `sender` is
        // optional (a DM's author is the chat itself); an empty `message_ids`
        // acknowledges nothing and the library emits no stanza.
        let sender = Jid::parse_optional(&req.sender.unwrap_or_default().value)?;
        messaging::mark_read(&client, &chat, sender.as_ref(), &req.message_ids).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn mark_chat_read(
        &self,
        request: Request<pb::MarkReadRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let chat = require_typed_jid(req.chat)?;
        chat_actions::mark_chat_read(&client, &chat).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn mark_unread(
        &self,
        request: Request<pb::MarkReadRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let chat = require_typed_jid(req.chat)?;
        chat_actions::mark_unread(&client, &chat).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn star_message(
        &self,
        request: Request<pb::StarMessageRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let target = MessageTarget::try_from(req.target.ok_or_else(|| missing_field("target"))?)?;
        chat_actions::star_message(&client, &target, req.starred).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn archive_chat(
        &self,
        request: Request<pb::ArchiveChatRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let chat = require_typed_jid(req.chat)?;
        chat_actions::archive_chat(&client, chat, req.archived).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn pin_chat(
        &self,
        request: Request<pb::PinChatRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let chat = require_typed_jid(req.chat)?;
        chat_actions::pin_chat(&client, chat, req.pinned).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn mute_chat(
        &self,
        request: Request<pb::MuteChatRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let chat = require_typed_jid(req.chat)?;
        chat_actions::mute_chat(&client, chat, req.muted, req.mute_until_ms).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn delete_chat(
        &self,
        request: Request<pb::DeleteChatRequest>,
    ) -> Result<Response<pb::Empty>, Status> {
        let req = request.into_inner();
        let client = client_of(&self.registry, req.account.as_ref()).await?;
        let chat = require_typed_jid(req.chat)?;
        chat_actions::delete_chat(&client, chat, req.delete_media).await?;
        Ok(Response::new(pb::Empty {}))
    }

    async fn send_contact(
        &self,
        request: Request<pb::SendContactRequest>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let req = request.into_inner();
        let (handle, client) = account_of(&self.registry, req.account.as_ref()).await?;
        let to = require_typed_jid(req.to.clone())?;
        let card = ContactCard::try_from(req)?;
        let result = send_rich::send_contact(&client, to, &card).await?;
        Ok(self.echoed(&handle, &client, result).await)
    }

    async fn send_interactive_reply(
        &self,
        request: Request<pb::SendInteractiveReplyRequest>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let req = request.into_inner();
        let (handle, client) = account_of(&self.registry, req.account.as_ref()).await?;
        let to = require_typed_jid(req.to.clone())?;
        let reply = InteractiveReply::try_from(req)?;
        let result = interactive_reply::send_interactive_reply(&client, to, &reply).await?;
        Ok(self.echoed(&handle, &client, result).await)
    }

    async fn send_poll(
        &self,
        request: Request<pb::SendPollRequest>,
    ) -> Result<Response<pb::SendPollResult>, Status> {
        let req = request.into_inner();
        let (handle, client) = account_of(&self.registry, req.account.as_ref()).await?;
        let to = require_typed_jid(req.to.clone())?;
        let poll = NewPoll::from(req);
        let (result, message_secret) = send_rich::send_poll(&client, to, &poll).await?;
        self.echo(&handle, &client, &result).await;
        // The poll key is the same key every send answers with; the
        // message_secret rides alongside so the edge can decrypt incoming votes.
        Ok(Response::new(pb::SendPollResult {
            key: Some(sent_message_key(result.message_id, &result.to)),
            message_secret,
            recipient_fanout: result.recipient_fanout.map(recipient_fanout_to_proto),
        }))
    }

    async fn send_poll_vote(
        &self,
        request: Request<pb::SendPollVoteRequest>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let req = request.into_inner();
        let (handle, client) = account_of(&self.registry, req.account.as_ref()).await?;
        let chat = require_typed_jid(req.chat.clone())?;
        let vote = PollVoteCast::try_from(req)?;
        let result = polls::send_vote(&client, chat, &vote).await?;
        Ok(self.echoed(&handle, &client, result).await)
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
        let tally = PollVotesToTally::try_from(req)?;
        Ok(Response::new(
            polls::aggregate_votes(&client, &tally).await?,
        ))
    }

    async fn post_status_text(
        &self,
        request: Request<pb::PostStatusTextRequest>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let req = request.into_inner();
        let (handle, client) = account_of(&self.registry, req.account.as_ref()).await?;
        let post = StatusText::try_from(req)?;
        let result = status::post_status_text(&client, &post).await?;
        Ok(self.echoed(&handle, &client, result).await)
    }

    async fn post_status_media(
        &self,
        request: Request<Streaming<pb::PostStatusMediaChunk>>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let mut chunks = MediaChunks::Status(request.into_inner());
        let header = chunks
            .read_header("empty status media stream")
            .await?
            .into_status()?;
        let (handle, client) = account_of(&self.registry, header.account.as_ref()).await?;
        // Same inline-only contract as SendMedia: the core fetches no URLs.
        let data = chunks.collect_bytes(self.media_max_bytes).await?;
        let post = StatusMedia::try_from(header)?;
        let result = status::post_status_media(&client, &post, data).await?;
        Ok(self.echoed(&handle, &client, result).await)
    }

    async fn revoke_status(
        &self,
        request: Request<pb::RevokeStatusRequest>,
    ) -> Result<Response<pb::SendResult>, Status> {
        let req = request.into_inner();
        // Shape first, account second (#41): a malformed revoke is about the
        // request, not the account.
        let account = req.account.clone();
        let revoke = StatusRevoke::try_from(req)?;
        let (handle, client) = account_of(&self.registry, account.as_ref()).await?;
        let result = status::revoke_status(&client, revoke).await?;
        Ok(self.echoed(&handle, &client, result).await)
    }
}
