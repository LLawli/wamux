//! Build and send chat messages. whatsapp-rust has no convenience senders for
//! normal chats, so we construct `wa::Message` and call `send_message`.

use std::sync::Arc;

use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::{Client, Jid, RevokeType, SendResult};

use crate::domain::jid_parse::parse_jid;
use crate::error::WamuxError;
use crate::proto::v1 as pb;

fn client_err<E: std::fmt::Display>(e: E) -> WamuxError {
    // `{:#}` yields anyhow's full cause chain (plain Display for other errors).
    WamuxError::Client(format!("{e:#}"))
}

// The core does NOT rewrite recipient JIDs. Routing (e.g. sending via `@c.us`
// vs `@s.whatsapp.net` to work around the library's PN->LID upgrade) is the
// edge's responsibility: it passes the exact JID it wants and the core relays
// to it verbatim. Keeping this transport-pure is a deliberate design choice.

pub async fn send_text(
    client: &Client,
    to: Jid,
    text: &str,
    mentions: &[pb::Mention],
    quote: Option<&pb::QuoteContext>,
) -> Result<SendResult, WamuxError> {
    let message = if mentions.is_empty() && quote.is_none() {
        wa::Message {
            conversation: Some(text.to_string()),
            ..Default::default()
        }
    } else {
        let mut context = wa::ContextInfo::default();
        if !mentions.is_empty() {
            context.mentioned_jid = mentions.iter().map(|m| m.jid.clone()).collect();
        }
        if let Some(q) = quote
            && let Some(key) = &q.quoted
        {
            context.stanza_id = Some(key.id.clone());
            let participant = if q.participant.is_empty() {
                key.participant.clone()
            } else {
                q.participant.clone()
            };
            context.participant = Some(participant);
        }
        wa::Message {
            extended_text_message: Some(Box::new(wa::message::ExtendedTextMessage {
                text: Some(text.to_string()),
                context_info: Some(Box::new(context)),
                ..Default::default()
            })),
            ..Default::default()
        }
    };
    client.send_message(to, message).await.map_err(client_err)
}

pub async fn send_reaction(
    client: &Client,
    target: &pb::MessageKey,
    emoji: &str,
) -> Result<SendResult, WamuxError> {
    let to = parse_jid(&target.remote_jid)?;
    let key = proto_key_to_wa(target);
    let message = wa::Message {
        reaction_message: Some(wa::message::ReactionMessage {
            key: Some(key),
            text: Some(emoji.to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    client.send_message(to, message).await.map_err(client_err)
}

pub async fn edit_message(
    client: &Client,
    target: &pb::MessageKey,
    new_text: &str,
) -> Result<String, WamuxError> {
    let to = parse_jid(&target.remote_jid)?;
    let new = wa::Message {
        conversation: Some(new_text.to_string()),
        ..Default::default()
    };
    client
        .edit_message(to, target.id.clone(), new)
        .await
        .map_err(client_err)
}

pub async fn delete_message(
    client: &Client,
    target: &pb::MessageKey,
    for_everyone: bool,
) -> Result<(), WamuxError> {
    let to = parse_jid(&target.remote_jid)?;
    if for_everyone {
        client
            .revoke_message(to, target.id.clone(), RevokeType::Sender)
            .await
            .map_err(client_err)
    } else {
        client
            .chat_actions()
            .delete_message_for_me(&to, None, &target.id, target.from_me, false, None)
            .await
            .map_err(client_err)
    }
}

/// Request on-demand message history (PDO HistorySyncOnDemand): the phone returns
/// `count` messages older than the anchor. Returns a session id that correlates
/// with the eventual `HistorySyncEvent.session_id` on the event stream.
pub async fn fetch_message_history(
    client: &Arc<Client>,
    chat: Jid,
    oldest_msg_id: &str,
    oldest_msg_from_me: bool,
    oldest_msg_timestamp_ms: i64,
    count: i32,
) -> Result<String, WamuxError> {
    client
        .fetch_message_history(
            &chat,
            oldest_msg_id,
            oldest_msg_from_me,
            oldest_msg_timestamp_ms,
            count,
        )
        .await
        .map_err(client_err)
}

pub async fn send_presence(client: &Client, chat: Jid, state: &str) -> Result<(), WamuxError> {
    match state {
        "available" => client.presence().set_available().await.map_err(client_err),
        "unavailable" => client
            .presence()
            .set_unavailable()
            .await
            .map_err(client_err),
        "composing" => client
            .chatstate()
            .send_composing(&chat)
            .await
            .map_err(client_err),
        "recording" => client
            .chatstate()
            .send_recording(&chat)
            .await
            .map_err(client_err),
        "paused" => client
            .chatstate()
            .send_paused(&chat)
            .await
            .map_err(client_err),
        other => Err(WamuxError::InvalidArgument(format!(
            "unknown presence state '{other}'"
        ))),
    }
}

pub async fn mark_read(client: &Client, chat: &Jid) -> Result<(), WamuxError> {
    client
        .chat_actions()
        .mark_chat_as_read(chat, true, None)
        .await
        .map_err(client_err)
}

fn proto_key_to_wa(key: &pb::MessageKey) -> wa::MessageKey {
    wa::MessageKey {
        remote_jid: Some(key.remote_jid.clone()),
        id: Some(key.id.clone()),
        from_me: Some(key.from_me),
        participant: if key.participant.is_empty() {
            None
        } else {
            Some(key.participant.clone())
        },
    }
}

pub fn send_result_to_proto(result: SendResult) -> pb::SendResult {
    pb::SendResult {
        key: Some(pb::MessageKey {
            remote_jid: result.to.to_string(),
            id: result.message_id,
            from_me: true,
            participant: String::new(),
        }),
        server_timestamp: 0,
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn send_result_maps_to_proto_key_with_from_me() {
        let result = SendResult {
            message_id: "3EB0ABCDEF".to_string(),
            to: Jid::from_str("5511999999999@s.whatsapp.net").unwrap(),
        };
        let proto = send_result_to_proto(result);
        let key = proto.key.expect("key must be set");
        assert_eq!(key.remote_jid, "5511999999999@s.whatsapp.net");
        assert_eq!(key.id, "3EB0ABCDEF");
        assert!(key.from_me);
        assert!(key.participant.is_empty());
        // The lib's SendResult carries no server timestamp; we pin 0 so the
        // edge knows the field is a placeholder, not a real clock reading.
        assert_eq!(proto.server_timestamp, 0);
    }

    #[test]
    fn proto_key_with_participant_maps_to_some() {
        let key = pb::MessageKey {
            remote_jid: "120363001234567890@g.us".to_string(),
            id: "MSG-1".to_string(),
            from_me: false,
            participant: "5511888888888@s.whatsapp.net".to_string(),
        };
        let wa_key = proto_key_to_wa(&key);
        assert_eq!(
            wa_key.remote_jid.as_deref(),
            Some("120363001234567890@g.us")
        );
        assert_eq!(wa_key.id.as_deref(), Some("MSG-1"));
        assert_eq!(wa_key.from_me, Some(false));
        assert_eq!(
            wa_key.participant.as_deref(),
            Some("5511888888888@s.whatsapp.net")
        );
    }

    // Proto3 has no optional string here: empty string is the wire encoding
    // of "no participant", so it must become None, never Some("").
    #[test]
    fn proto_key_empty_participant_maps_to_none() {
        let key = pb::MessageKey {
            remote_jid: "5511999999999@s.whatsapp.net".to_string(),
            id: "MSG-2".to_string(),
            from_me: true,
            participant: String::new(),
        };
        let wa_key = proto_key_to_wa(&key);
        assert_eq!(wa_key.participant, None);
        assert_eq!(wa_key.from_me, Some(true));
    }
}
