//! `messaging` (#115): the tests that lived inline, moved to their own file so
//! the loop can seal them. Same assertions; only the inputs changed type, from
//! the wire's `pb::*` to the domain's own.

use std::str::FromStr;

use wacore::send::RecipientFanout;
use wamux_types::{
    Jid, LinkPreview, MessageId, MessageTarget, OutgoingContext, OutgoingText, QuotedRef,
};
use whatsapp_rust::waproto::whatsapp::message::extended_text_message::PreviewType;

use super::{
    build_text_message, recipient_fanout_to_proto, refuse_status_revoke, send_result_to_proto,
    wa_message_key,
};
use crate::error::WamuxError;
use crate::proto::v1 as pb;

fn jid(value: &str) -> Jid {
    Jid::parse(value).unwrap()
}

fn id(value: &str) -> MessageId {
    MessageId::new(value).unwrap()
}

fn target_in(chat: &str) -> MessageTarget {
    MessageTarget {
        chat: jid(chat),
        id: id("3EB0TARGET"),
        from_me: true,
        participant: None,
    }
}

/// Issue #41: only a revoke of a status is refused; deleting a status for
/// me, and revoking anything in a real chat, still go through.
#[test]
fn only_a_status_revoke_is_refused() {
    let status = target_in("status@broadcast");
    assert!(matches!(
        refuse_status_revoke(&status, true),
        Err(WamuxError::InvalidArgument(_))
    ));
    assert!(refuse_status_revoke(&status, false).is_ok());
    for chat in ["5511999000111@s.whatsapp.net", "120363041234567890@g.us"] {
        assert!(
            refuse_status_revoke(&target_in(chat), true).is_ok(),
            "{chat}"
        );
    }
}

#[test]
fn send_result_maps_to_proto_key_with_from_me() {
    let proto = send_result_to_proto(
        "3EB0ABCDEF".to_string(),
        &whatsapp_rust::Jid::from_str("5511999999999@s.whatsapp.net").unwrap(),
        None,
    );
    let key = proto.key.expect("key must be set");
    assert_eq!(
        key.chat,
        Some(pb::Jid {
            value: "5511999999999@s.whatsapp.net".to_string()
        })
    );
    assert_eq!(key.id, "3EB0ABCDEF");
    assert!(key.from_me);
    // A send has no participant: unset, never `Jid { value: "" }` (#120).
    assert_eq!(key.participant, None);
    // The lib's SendResult carries no server timestamp; we pin 0 so the
    // edge knows the field is a placeholder, not a real clock reading.
    assert_eq!(proto.server_timestamp, 0);
}

// Issue #47: the four facts cross as the library counted them.
#[test]
fn a_dm_send_relays_its_recipient_fanout() {
    let mut fanout = RecipientFanout::default();
    fanout.addressed = 3;
    fanout.encrypted = 2;
    fanout.skipped_primary = true;
    fanout.had_unregistered_device = true;
    let proto = send_result_to_proto(
        "3EB0ABCDEF".to_string(),
        &whatsapp_rust::Jid::from_str("5511999999999@s.whatsapp.net").unwrap(),
        Some(fanout),
    );
    let relayed = proto.recipient_fanout.expect("a DM carries its fan-out");
    assert_eq!((relayed.addressed, relayed.encrypted), (3, 2));
    assert!(relayed.skipped_primary);
    assert!(relayed.had_unregistered_device);
}

// A self-chat is zeros, present; a non-DM send is absent. The two must not
// collapse into one another on the wire.
#[test]
fn a_self_chat_fanout_stays_apart_from_a_non_dm_send() {
    let to = whatsapp_rust::Jid::from_str("5511999999999@s.whatsapp.net").unwrap();
    let self_chat = send_result_to_proto("A".to_string(), &to, Some(RecipientFanout::default()));
    assert_eq!(
        self_chat.recipient_fanout,
        Some(pb::RecipientFanout::default())
    );
    let group = send_result_to_proto("B".to_string(), &to, None);
    assert_eq!(group.recipient_fanout, None);
}

#[test]
fn a_device_count_past_u32_saturates() {
    let mut fanout = RecipientFanout::default();
    fanout.addressed = usize::MAX;
    assert_eq!(recipient_fanout_to_proto(fanout).addressed, u32::MAX);
}

#[test]
fn target_with_participant_maps_to_some() {
    let target = MessageTarget {
        chat: jid("120363001234567890@g.us"),
        id: id("MSG-1"),
        from_me: false,
        participant: Some(jid("5511888888888@s.whatsapp.net")),
    };
    let wa_key = wa_message_key(&target);
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

// No participant is the DM case: it must stay the absent field, never Some("").
#[test]
fn target_without_participant_maps_to_none() {
    let target = MessageTarget {
        chat: jid("5511999999999@s.whatsapp.net"),
        id: id("MSG-2"),
        from_me: true,
        participant: None,
    };
    let wa_key = wa_message_key(&target);
    assert_eq!(wa_key.participant, None);
    assert_eq!(wa_key.from_me, Some(true));
}

fn full_preview() -> LinkPreview {
    LinkPreview {
        matched_text: "https://example.com/post".to_string(),
        title: "A title".to_string(),
        description: "A description".to_string(),
        jpeg_thumbnail: vec![0xff, 0xd8, 0xff],
        preview_type: 1, // VIDEO
    }
}

fn text(text: &str) -> OutgoingText {
    OutgoingText {
        text: text.to_string(),
        ..Default::default()
    }
}

fn quote(quoted_id: &str, participant: Option<&str>) -> Option<QuotedRef> {
    Some(QuotedRef {
        id: id(quoted_id),
        participant: participant.map(jid),
    })
}

#[test]
fn plain_text_stays_conversation() {
    let message = build_text_message(&text("oi")).unwrap();
    assert_eq!(message.conversation.as_deref(), Some("oi"));
    assert!(message.extended_text_message.is_unset());
}

#[test]
fn link_preview_forces_extended_with_fields_relayed_verbatim() {
    let message = build_text_message(&OutgoingText {
        link_preview: Some(full_preview()),
        ..text("look https://example.com/post")
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
    let preview = LinkPreview {
        matched_text: "https://example.com".to_string(),
        title: String::new(),
        description: String::new(),
        jpeg_thumbnail: vec![],
        preview_type: 0,
    };
    let ext = build_text_message(&OutgoingText {
        link_preview: Some(preview),
        ..text("https://example.com")
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

// waproto 0.7 types preview_type as a closed enum: an out-of-schema number is
// refused, not dropped (the socket suite pins the same over the wire).
#[test]
fn an_unknown_preview_type_is_invalid_argument() {
    let preview = LinkPreview {
        preview_type: 999,
        ..full_preview()
    };
    let err = build_text_message(&OutgoingText {
        link_preview: Some(preview),
        ..text("https://example.com")
    })
    .expect_err("999 is no PreviewType");
    match err {
        WamuxError::InvalidArgument(msg) => assert!(msg.contains("999"), "got: {msg}"),
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

// Regression (code-review 2026-06-11): a preview-only extended text must
// NOT carry a present-but-empty ContextInfo — regular clients send the
// field absent, and an empty submessage is a fingerprintable wire shape.
#[test]
fn link_preview_only_leaves_context_absent() {
    let ext = build_text_message(&OutgoingText {
        link_preview: Some(full_preview()),
        ..text("https://example.com")
    })
    .unwrap()
    .extended_text_message
    .expect("preview presence alone must force extended");
    assert!(ext.context_info.is_unset());
}

// Regression (code-review 2026-06-11): quoting in a DM has no participant;
// that must relay as the absent field, never Some("") — an empty JID on the
// WhatsApp wire.
#[test]
fn dm_quote_without_participant_maps_participant_to_none() {
    let ext = build_text_message(&OutgoingText {
        context: OutgoingContext {
            quote: quote("QUOTED-DM", None),
            ..Default::default()
        },
        ..text("re: that")
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
    let message = build_text_message(&OutgoingText {
        context: OutgoingContext {
            ephemeral_seconds: 86_400,
            ..Default::default()
        },
        ..text("fugaz")
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
    let message = build_text_message(&OutgoingText {
        context: OutgoingContext {
            mentions: vec![jid("5511888888888@s.whatsapp.net")],
            quote: quote("QUOTED-1", Some("5511777777777@s.whatsapp.net")),
            ephemeral_seconds: 90,
        },
        link_preview: Some(full_preview()),
        ..text("all of it")
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
    let ext = build_text_message(&OutgoingText {
        context: OutgoingContext {
            mentions: vec![jid("5511888888888@s.whatsapp.net")],
            ..Default::default()
        },
        ..text("@you")
    })
    .unwrap()
    .extended_text_message
    .expect("mentions force extended");
    let context = ext.context_info.expect("context_info must be set");
    assert_eq!(context.expiration, None);
}
