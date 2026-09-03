//! Build and send chat messages. whatsapp-rust has no convenience senders for
//! normal chats, so we construct `wa::Message` and call `send_message`.

use std::sync::Arc;

use whatsapp_rust::buffa::{Enumeration, MessageField};
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::waproto::whatsapp::message::extended_text_message::PreviewType;
use whatsapp_rust::{Client, Jid, RevokeType, SendResult};

use crate::domain::jid_parse::parse_jid;
use crate::domain::outgoing_context::outgoing_context;
use crate::domain::wire_defaults::{nonempty_bytes, nonempty_string, nonzero_i32};
use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

// The core does NOT rewrite recipient JIDs. Routing (e.g. sending via `@c.us`
// vs `@s.whatsapp.net` to work around the library's PN->LID upgrade) is the
// edge's responsibility: it passes the exact JID it wants and the core relays
// to it verbatim. Keeping this transport-pure is a deliberate design choice.

/// Wire-shaped like the media path (`send_media` + its header): the routing
/// fields `req.account`/`req.to` were already consumed by the caller and are
/// ignored here; every content field relays through `build_text_message`, so
/// a new proto field touches only the builder, never this signature or its
/// call sites (code-review 2026-06-11).
pub async fn send_text(
    client: &Client,
    to: Jid,
    req: &pb::SendTextRequest,
) -> Result<(SendResult, wa::Message), WamuxError> {
    let message = build_text_message(req)?;
    // The built message rides back out so the service can echo it (issue #22):
    // WhatsApp never echoes a send back to the device that made it, and the
    // echo must carry the bytes that actually went, not a reconstruction.
    let result = client
        .send_message(to, message.clone())
        .await
        .map_err(client_err)?;
    Ok((result, message))
}

/// Pure construction of the outgoing text `wa::Message`. Plain `conversation`
/// only when nothing extra rides along; any mention/quote/preview/ephemeral
/// upgrades it to an `ExtendedTextMessage`. Everything is relayed verbatim:
/// the EDGE fetched the preview and chose the expiration (the core does no
/// outbound HTTP and tracks no chat settings).
pub(crate) fn build_text_message(req: &pb::SendTextRequest) -> Result<wa::Message, WamuxError> {
    let plain = req.mentions.is_empty()
        && req.quote.is_none()
        && req.link_preview.is_none()
        && req.ephemeral_seconds == 0;
    if plain {
        return Ok(wa::Message {
            conversation: Some(req.text.clone()),
            ..Default::default()
        });
    }
    Ok(wa::Message {
        extended_text_message: MessageField::some(extended_text(req)?),
        ..Default::default()
    })
}

fn extended_text(
    req: &pb::SendTextRequest,
) -> Result<wa::message::ExtendedTextMessage, WamuxError> {
    let mut extended = wa::message::ExtendedTextMessage {
        text: Some(req.text.clone()),
        // Unset for a preview-only message: shared builder, see outgoing_context.
        context_info: outgoing_context(&req.mentions, req.quote.as_ref(), req.ephemeral_seconds),
        ..Default::default()
    };
    if let Some(preview) = &req.link_preview {
        copy_link_preview(&mut extended, preview)?;
    }
    Ok(extended)
}

/// Relay the edge-supplied preview verbatim onto the extended text. This
/// waproto has no canonical_url: matched_text IS the URL. preview_type 0
/// (NONE) is both the proto3 default and the wa default, so it relays as the
/// absent field, the same lib-natural form a regular link preview uses.
fn copy_link_preview(
    extended: &mut wa::message::ExtendedTextMessage,
    preview: &pb::LinkPreview,
) -> Result<(), WamuxError> {
    extended.matched_text = nonempty_string(&preview.matched_text);
    extended.title = nonempty_string(&preview.title);
    extended.description = nonempty_string(&preview.description);
    extended.jpeg_thumbnail = nonempty_bytes(&preview.jpeg_thumbnail);
    // waproto 0.7 types this as a closed enum, so an out-of-schema number can
    // no longer ride the wire the way prost's `Option<i32>` let it. Reject it
    // instead of silently dropping the field: dropping would change what the
    // edge asked to relay without telling anyone.
    extended.preview_type = match nonzero_i32(preview.preview_type) {
        None => None,
        Some(value) => Some(PreviewType::from_i32(value).ok_or_else(|| {
            WamuxError::InvalidArgument(format!(
                "unknown link_preview.preview_type {value}; expected a wa PreviewType value"
            ))
        })?),
    };
    Ok(())
}

pub async fn send_reaction(
    client: &Client,
    target: &pb::MessageKey,
    emoji: &str,
) -> Result<(SendResult, wa::Message), WamuxError> {
    let to = parse_jid(&target.remote_jid)?;
    let key = proto_key_to_wa(target);
    let message = wa::Message {
        reaction_message: MessageField::some(wa::message::ReactionMessage {
            key: MessageField::some(key),
            text: Some(emoji.to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let result = client
        .send_message(to, message.clone())
        .await
        .map_err(client_err)?;
    Ok((result, message))
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

/// Send a read receipt: the stanza that turns the other person's ticks blue.
///
/// Issue #20: this used to call `chat_actions().mark_chat_as_read()`, whose own
/// doc line says it is "distinct from `readMessages` IQ receipts -- this syncs
/// state across linked devices". So `MarkRead` marked the chat read on the
/// account's OWN devices, ignored `message_ids` and `sender`, and never put a
/// receipt on the wire. It returned `Empty` either way, which is why nothing
/// downstream noticed. That mutation still exists, under its own name, in
/// `chat_actions::mark_chat_read`.
///
/// Empty `message_ids` is not an error: the library returns before building a
/// stanza, so a caller with nothing to acknowledge costs nothing.
pub async fn mark_read(
    client: &Client,
    chat: &Jid,
    sender: Option<&Jid>,
    message_ids: &[String],
) -> Result<(), WamuxError> {
    let ids: Vec<&str> = message_ids.iter().map(String::as_str).collect();
    client
        .mark_as_read(chat, sender, &ids)
        .await
        .map_err(client_err)
}

fn proto_key_to_wa(key: &pb::MessageKey) -> wa::MessageKey {
    wa::MessageKey {
        remote_jid: Some(key.remote_jid.clone()),
        id: Some(key.id.clone()),
        from_me: Some(key.from_me),
        participant: nonempty_string(&key.participant),
    }
}

/// Takes the two fields instead of the whole `SendResult`: 0.7 sealed that
/// struct (`#[non_exhaustive]`, no public constructor), so a projection that
/// consumed it could not be exercised from a test at all.
pub fn send_result_to_proto(message_id: String, to: &Jid) -> pb::SendResult {
    pb::SendResult {
        key: Some(pb::MessageKey {
            remote_jid: to.to_string(),
            id: message_id,
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
        let proto = send_result_to_proto(
            "3EB0ABCDEF".to_string(),
            &Jid::from_str("5511999999999@s.whatsapp.net").unwrap(),
        );
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

    /// Content-only request: routing fields stay at their (ignored) defaults.
    fn text_req(text: &str) -> pb::SendTextRequest {
        pb::SendTextRequest {
            text: text.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn plain_text_stays_conversation() {
        let message = build_text_message(&text_req("oi")).unwrap();
        assert_eq!(message.conversation.as_deref(), Some("oi"));
        assert!(message.extended_text_message.is_unset());
    }

    #[test]
    fn link_preview_forces_extended_with_fields_relayed_verbatim() {
        let message = build_text_message(&pb::SendTextRequest {
            link_preview: Some(full_preview()),
            ..text_req("look https://example.com/post")
        })
        .unwrap();
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
        assert_eq!(ext.preview_type, Some(PreviewType::VIDEO));
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
        let ext = build_text_message(&pb::SendTextRequest {
            link_preview: Some(preview),
            ..text_req("https://example.com")
        })
        .unwrap()
        .extended_text_message
        .expect("preview presence alone must force extended");
        assert_eq!(ext.matched_text.as_deref(), Some("https://example.com"));
        assert_eq!(ext.title, None);
        assert_eq!(ext.description, None);
        assert_eq!(ext.jpeg_thumbnail, None);
        assert_eq!(ext.preview_type, None);
    }

    // Regression (code-review 2026-06-11): a preview-only extended text must
    // NOT carry a present-but-empty ContextInfo — regular clients send the
    // field absent, and an empty submessage is a fingerprintable wire shape.
    #[test]
    fn link_preview_only_leaves_context_absent() {
        let ext = build_text_message(&pb::SendTextRequest {
            link_preview: Some(full_preview()),
            ..text_req("https://example.com")
        })
        .unwrap()
        .extended_text_message
        .expect("preview presence alone must force extended");
        assert!(ext.context_info.is_unset());
    }

    // Regression (code-review 2026-06-11): quoting in a DM leaves both
    // participant fields empty; that must relay as the absent field, never
    // Some("") — an empty JID on the WhatsApp wire.
    #[test]
    fn dm_quote_with_empty_participants_maps_participant_to_none() {
        let quote = pb::QuoteContext {
            quoted: Some(pb::MessageKey {
                remote_jid: "5511999999999@s.whatsapp.net".to_string(),
                id: "QUOTED-DM".to_string(),
                from_me: false,
                participant: String::new(),
            }),
            participant: String::new(),
        };
        let ext = build_text_message(&pb::SendTextRequest {
            quote: Some(quote),
            ..text_req("re: that")
        })
        .unwrap()
        .extended_text_message
        .expect("quote forces extended");
        let context = ext.context_info.expect("quote must build a context");
        assert_eq!(context.stanza_id.as_deref(), Some("QUOTED-DM"));
        assert_eq!(context.participant, None);
    }

    #[test]
    fn ephemeral_text_sets_context_expiration() {
        let message = build_text_message(&pb::SendTextRequest {
            ephemeral_seconds: 86_400,
            ..text_req("fugaz")
        })
        .unwrap();
        let ext = message.extended_text_message.expect("must be extended");
        let context = ext.context_info.expect("context_info must be set");
        assert_eq!(context.expiration, Some(86_400));
        // Nothing else rode along: no mentions, no quote.
        assert!(context.mentioned_jid.is_empty());
        assert_eq!(context.stanza_id, None);
    }

    #[test]
    fn preview_mentions_quote_and_ephemeral_compose_in_one_extended() {
        let message = build_text_message(&pb::SendTextRequest {
            mentions: vec![pb::Mention {
                jid: "5511888888888@s.whatsapp.net".to_string(),
            }],
            quote: Some(pb::QuoteContext {
                quoted: Some(pb::MessageKey {
                    remote_jid: "120363001234567890@g.us".to_string(),
                    id: "QUOTED-1".to_string(),
                    from_me: false,
                    participant: "5511777777777@s.whatsapp.net".to_string(),
                }),
                participant: String::new(),
            }),
            link_preview: Some(full_preview()),
            ephemeral_seconds: 90,
            ..text_req("all of it")
        })
        .unwrap();
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
        let ext = build_text_message(&pb::SendTextRequest {
            mentions: vec![pb::Mention {
                jid: "5511888888888@s.whatsapp.net".to_string(),
            }],
            ..text_req("@you")
        })
        .unwrap()
        .extended_text_message
        .expect("mentions force extended");
        let context = ext.context_info.expect("context_info must be set");
        assert_eq!(context.expiration, None);
    }
}
