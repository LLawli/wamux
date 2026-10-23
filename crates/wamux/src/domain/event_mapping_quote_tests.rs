//! #217: the quote and mentions of an inbound message, read from whichever
//! message type carries the context, and the quoted key's chat taken from the
//! context's `remoteJid` when it names one. The private-reply case is the one
//! wamux-omarchy measured: 7 of 11 replies with a `remoteJid` named a group,
//! and every one arrived naming the direct chat instead.

#![allow(clippy::field_reassign_with_default)] // waproto structs are built by assignment

use whatsapp_rust::buffa::MessageField;

use super::*;
use wacore::types::message::MessageSource;

const DM: &str = "5511999000217@s.whatsapp.net";
const GROUP: &str = "120363041234567217@g.us";
const AUTHOR: &str = "5511888000217@s.whatsapp.net";
const MENTIONED: &str = "5511777000217@s.whatsapp.net";
const STATUS: &str = "status@broadcast";

fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

/// A message received in the direct chat with `DM`.
fn dm_info() -> MessageInfo {
    MessageInfo {
        source: MessageSource {
            chat: DM.parse().unwrap(),
            sender: DM.parse().unwrap(),
            is_from_me: false,
            ..Default::default()
        },
        id: "REPLY-217".into(),
        timestamp: wacore::time::from_secs(1_792_000_000).unwrap(),
        ..Default::default()
    }
}

/// A context quoting `QUOTED-217` by `AUTHOR`, mentioning `MENTIONED`.
fn quoting(remote_jid: Option<&str>) -> wa::ContextInfo {
    let mut context = wa::ContextInfo::default();
    context.stanza_id = Some("QUOTED-217".to_string());
    context.participant = Some(AUTHOR.to_string());
    context.remote_jid = remote_jid.map(str::to_string);
    context.mentioned_jid = vec![MENTIONED.to_string()];
    context
}

fn text_reply(context: wa::ContextInfo) -> wa::Message {
    let mut text = wa::message::ExtendedTextMessage::default();
    text.text = Some("resposta".to_string());
    text.context_info = MessageField::some(context);
    let mut message = wa::Message::default();
    message.extended_text_message = MessageField::some(text);
    message
}

/// The library hands both halves over as `Arc`s, as `map_event` receives them.
fn mapped(message: &wa::Message) -> pb::InboundMessage {
    map_message(&Arc::new(message.clone()), &Arc::new(dm_info()))
}

fn quoted_key(message: &wa::Message) -> pb::MessageKey {
    let out = mapped(message);
    out.quote
        .expect("a context with a stanza id is a quote")
        .quoted
        .expect("the quote carries the quoted key")
}

#[test]
fn private_reply_quotes_the_chat_remote_jid_names() {
    let key = quoted_key(&text_reply(quoting(Some(GROUP))));
    assert_eq!(key.chat, wire(GROUP), "the quoted message is in the group");
    assert_eq!(key.id, "QUOTED-217");
    assert_eq!(key.participant, wire(AUTHOR));
}

#[test]
fn reply_without_remote_jid_quotes_its_own_chat() {
    assert_eq!(quoted_key(&text_reply(quoting(None))).chat, wire(DM));
}

#[test]
fn empty_remote_jid_is_the_reply_chat() {
    assert_eq!(quoted_key(&text_reply(quoting(Some("")))).chat, wire(DM));
}

#[test]
fn status_reply_quotes_status_broadcast() {
    assert_eq!(
        quoted_key(&text_reply(quoting(Some(STATUS)))).chat,
        wire(STATUS)
    );
}

/// Every media sub-message carries its own context: a reply sent as an image,
/// a video, a document, a sticker or a voice note quotes like a text does.
#[test]
fn media_reply_carries_its_quote_and_mentions() {
    let mut cases: Vec<(&str, wa::Message)> = Vec::new();
    let mut image = wa::message::ImageMessage::default();
    image.context_info = MessageField::some(quoting(Some(GROUP)));
    let mut message = wa::Message::default();
    message.image_message = MessageField::some(image);
    cases.push(("image", message));
    let mut video = wa::message::VideoMessage::default();
    video.context_info = MessageField::some(quoting(Some(GROUP)));
    let mut message = wa::Message::default();
    message.video_message = MessageField::some(video);
    cases.push(("video", message));
    let mut document = wa::message::DocumentMessage::default();
    document.context_info = MessageField::some(quoting(Some(GROUP)));
    let mut message = wa::Message::default();
    message.document_message = MessageField::some(document);
    cases.push(("document", message));
    let mut sticker = wa::message::StickerMessage::default();
    sticker.context_info = MessageField::some(quoting(Some(GROUP)));
    let mut message = wa::Message::default();
    message.sticker_message = MessageField::some(sticker);
    cases.push(("sticker", message));
    let mut audio = wa::message::AudioMessage::default();
    audio.context_info = MessageField::some(quoting(Some(GROUP)));
    let mut message = wa::Message::default();
    message.audio_message = MessageField::some(audio);
    cases.push(("audio", message));

    for (kind, message) in cases {
        let out = mapped(&message);
        let quote = out.quote.unwrap_or_else(|| panic!("{kind}: no quote"));
        let key = quote.quoted.expect("quoted key");
        assert_eq!(key.id, "QUOTED-217", "{kind}");
        assert_eq!(key.chat, wire(GROUP), "{kind}");
        assert_eq!(
            out.mentions,
            [pb::Mention {
                jid: wire(MENTIONED)
            }],
            "{kind}: mentions ride on the same context"
        );
    }
}

/// A disappearing chat wraps the message; the context inside still counts.
#[test]
fn wrapped_reply_carries_its_quote() {
    let mut wrapper = wa::message::FutureProofMessage::default();
    wrapper.message = MessageField::some(text_reply(quoting(Some(GROUP))));
    let mut message = wa::Message::default();
    message.ephemeral_message = MessageField::some(wrapper);
    assert_eq!(quoted_key(&message).chat, wire(GROUP));
}
