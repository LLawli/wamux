//! Build and send chat messages. whatsapp-rust has no convenience senders for
//! normal chats, so we construct `wa::Message` and call `send_message`.

use std::sync::Arc;

use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::{Client, Jid, RevokeType, SendResult};

use crate::domain::jid_parse::parse_jid;
use crate::domain::wire_defaults::{nonempty_bytes, nonempty_string};
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
    link_preview: Option<&pb::LinkPreview>,
    ephemeral_seconds: u32,
) -> Result<SendResult, WamuxError> {
    let message = build_text_message(text, mentions, quote, link_preview, ephemeral_seconds);
    client.send_message(to, message).await.map_err(client_err)
}

/// Pure construction of the outgoing text `wa::Message`. Plain `conversation`
/// only when nothing extra rides along; any mention/quote/preview/ephemeral
/// upgrades it to an `ExtendedTextMessage`. Everything is relayed verbatim:
/// the EDGE fetched the preview and chose the expiration (the core does no
/// outbound HTTP and tracks no chat settings).
pub(crate) fn build_text_message(
    text: &str,
    mentions: &[pb::Mention],
    quote: Option<&pb::QuoteContext>,
    link_preview: Option<&pb::LinkPreview>,
    ephemeral_seconds: u32,
) -> wa::Message {
    let plain =
        mentions.is_empty() && quote.is_none() && link_preview.is_none() && ephemeral_seconds == 0;
    if plain {
        return wa::Message {
            conversation: Some(text.to_string()),
            ..Default::default()
        };
    }
    let extended = extended_text(text, mentions, quote, link_preview, ephemeral_seconds);
    wa::Message {
        extended_text_message: Some(Box::new(extended)),
        ..Default::default()
    }
}

fn extended_text(
    text: &str,
    mentions: &[pb::Mention],
    quote: Option<&pb::QuoteContext>,
    link_preview: Option<&pb::LinkPreview>,
    ephemeral_seconds: u32,
) -> wa::message::ExtendedTextMessage {
    let mut extended = wa::message::ExtendedTextMessage {
        text: Some(text.to_string()),
        context_info: Some(Box::new(text_context(mentions, quote, ephemeral_seconds))),
        ..Default::default()
    };
    if let Some(preview) = link_preview {
        copy_link_preview(&mut extended, preview);
    }
    extended
}

fn text_context(
    mentions: &[pb::Mention],
    quote: Option<&pb::QuoteContext>,
    ephemeral_seconds: u32,
) -> wa::ContextInfo {
    let mut context = wa::ContextInfo::default();
    if !mentions.is_empty() {
        context.mentioned_jid = mentions.iter().map(|m| m.jid.clone()).collect();
    }
    copy_quote(&mut context, quote);
    if ephemeral_seconds > 0 {
        context.expiration = Some(ephemeral_seconds);
    }
    context
}

fn copy_quote(context: &mut wa::ContextInfo, quote: Option<&pb::QuoteContext>) {
    let Some(q) = quote else { return };
    let Some(key) = &q.quoted else { return };
    context.stanza_id = Some(key.id.clone());
    let participant = if q.participant.is_empty() {
        key.participant.clone()
    } else {
        q.participant.clone()
    };
    context.participant = Some(participant);
}

/// Relay the edge-supplied preview verbatim onto the extended text. This
/// waproto has no canonical_url: matched_text IS the URL. preview_type 0
/// (NONE) is both the proto3 default and the wa default, so it relays as the
/// absent field, the same lib-natural form a regular link preview uses.
fn copy_link_preview(extended: &mut wa::message::ExtendedTextMessage, preview: &pb::LinkPreview) {
    extended.matched_text = nonempty_string(&preview.matched_text);
    extended.title = nonempty_string(&preview.title);
    extended.description = nonempty_string(&preview.description);
    extended.jpeg_thumbnail = nonempty_bytes(&preview.jpeg_thumbnail);
    extended.preview_type = (preview.preview_type != 0).then_some(preview.preview_type);
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

    fn full_preview() -> pb::LinkPreview {
        pb::LinkPreview {
            matched_text: "https://example.com/post".to_string(),
            title: "A title".to_string(),
            description: "A description".to_string(),
            jpeg_thumbnail: vec![0xff, 0xd8, 0xff],
            preview_type: 1, // VIDEO
        }
    }

    #[test]
    fn plain_text_stays_conversation() {
        let message = build_text_message("oi", &[], None, None, 0);
        assert_eq!(message.conversation.as_deref(), Some("oi"));
        assert!(message.extended_text_message.is_none());
    }

    #[test]
    fn link_preview_forces_extended_with_fields_relayed_verbatim() {
        let message = build_text_message(
            "look https://example.com/post",
            &[],
            None,
            Some(&full_preview()),
            0,
        );
        assert!(message.conversation.is_none());
        let ext = message.extended_text_message.expect("must be extended");
        assert_eq!(ext.text.as_deref(), Some("look https://example.com/post"));
        assert_eq!(
            ext.matched_text.as_deref(),
            Some("https://example.com/post")
        );
        assert_eq!(ext.title.as_deref(), Some("A title"));
        assert_eq!(ext.description.as_deref(), Some("A description"));
        assert_eq!(ext.jpeg_thumbnail.as_deref(), Some(&[0xff, 0xd8, 0xff][..]));
        assert_eq!(ext.preview_type, Some(1));
    }

    // Proto3 defaults inside a present LinkPreview (empty string/bytes,
    // preview_type 0=NONE) relay as ABSENT waproto fields, never Some("").
    #[test]
    fn link_preview_empty_fields_map_to_none() {
        let preview = pb::LinkPreview {
            matched_text: "https://example.com".to_string(),
            title: String::new(),
            description: String::new(),
            jpeg_thumbnail: vec![],
            preview_type: 0,
        };
        let ext = build_text_message("https://example.com", &[], None, Some(&preview), 0)
            .extended_text_message
            .expect("preview presence alone must force extended");
        assert_eq!(ext.matched_text.as_deref(), Some("https://example.com"));
        assert_eq!(ext.title, None);
        assert_eq!(ext.description, None);
        assert_eq!(ext.jpeg_thumbnail, None);
        assert_eq!(ext.preview_type, None);
    }

    #[test]
    fn ephemeral_text_sets_context_expiration() {
        let message = build_text_message("fugaz", &[], None, None, 86_400);
        let ext = message.extended_text_message.expect("must be extended");
        let context = ext.context_info.expect("context_info must be set");
        assert_eq!(context.expiration, Some(86_400));
        // Nothing else rode along: no mentions, no quote.
        assert!(context.mentioned_jid.is_empty());
        assert_eq!(context.stanza_id, None);
    }

    #[test]
    fn preview_mentions_quote_and_ephemeral_compose_in_one_extended() {
        let mentions = [pb::Mention {
            jid: "5511888888888@s.whatsapp.net".to_string(),
        }];
        let quote = pb::QuoteContext {
            quoted: Some(pb::MessageKey {
                remote_jid: "120363001234567890@g.us".to_string(),
                id: "QUOTED-1".to_string(),
                from_me: false,
                participant: "5511777777777@s.whatsapp.net".to_string(),
            }),
            participant: String::new(),
        };
        let message = build_text_message(
            "all of it",
            &mentions,
            Some(&quote),
            Some(&full_preview()),
            90,
        );
        let ext = message.extended_text_message.expect("must be extended");
        assert_eq!(
            ext.matched_text.as_deref(),
            Some("https://example.com/post")
        );
        let context = ext.context_info.expect("context_info must be set");
        assert_eq!(
            context.mentioned_jid,
            vec!["5511888888888@s.whatsapp.net".to_string()]
        );
        assert_eq!(context.stanza_id.as_deref(), Some("QUOTED-1"));
        // Quote participant falls back to the quoted key's participant.
        assert_eq!(
            context.participant.as_deref(),
            Some("5511777777777@s.whatsapp.net")
        );
        assert_eq!(context.expiration, Some(90));
    }

    // ephemeral_seconds == 0 means "not ephemeral": even when other context
    // exists, expiration must stay absent (the core invents no duration).
    #[test]
    fn zero_ephemeral_leaves_expiration_absent() {
        let mentions = [pb::Mention {
            jid: "5511888888888@s.whatsapp.net".to_string(),
        }];
        let ext = build_text_message("@you", &mentions, None, None, 0)
            .extended_text_message
            .expect("mentions force extended");
        let context = ext.context_info.expect("context_info must be set");
        assert_eq!(context.expiration, None);
    }
}
