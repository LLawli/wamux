//! The projection of a `wa::Message` onto an `InboundMessage`: text, mentions,
//! quote, reaction, edit and revoke flags, media. One projection for both
//! directions, a message WhatsApp delivered and the echo of one this relay sent
//! (issue #22), so an edge never sees two shapes for one concept.

use std::sync::Arc;

use wacore::proto_helpers::MessageExt as _;
use wacore::types::message::MessageInfo;
use wamux_types::{MediaKind, relay_jid, relay_optional_lib_jid};
use whatsapp_rust::buffa::Message as _;
use whatsapp_rust::waproto::whatsapp as wa;

use crate::domain::sticker_packs;
use crate::proto::v1 as pb;

pub(super) fn map_message(msg: &Arc<wa::Message>, info: &Arc<MessageInfo>) -> pb::InboundMessage {
    let chat = info.source.chat.to_string();
    let sender = info.source.sender.to_string();
    let key = pb::MessageKey {
        chat: relay_jid(chat.clone()),
        id: info.id.to_string(),
        from_me: info.source.is_from_me,
        // The inbound participant is the sender even in a DM (#120).
        participant: relay_jid(sender.clone()),
    };

    let mut out = pb::InboundMessage {
        key: Some(key),
        chat: relay_jid(chat.clone()),
        sender: relay_jid(sender),
        timestamp: info.timestamp.as_millis(),
        // The library's `push_name` is a `CompactString` (#30); the wire
        // contract wants an owned `String`.
        push_name: info.push_name.to_string(),
        // The parser already put the stanza's other-namespace jids here
        // (sender_pn/participant_pn/participant_lid). Dropping them would force
        // the edge to poll for an identity the event itself carries (issue #1);
        // relaying them is verbatim, no lookup and no guess.
        // Unset when the stanza carried no alt jid, instead of an empty string
        // a consumer would have to know to ignore (#122).
        sender_alt: relay_optional_lib_jid(info.source.sender_alt.as_ref()),
        recipient_alt: relay_optional_lib_jid(info.source.recipient_alt.as_ref()),
        raw_message: msg.encode_to_vec(),
        ..Default::default()
    };

    project_content(&mut out, msg, &chat);
    out
}

/// Mentions of a context, dropping an empty one, never `Mention { jid: None }` (#120).
fn mentions_of(ci: &wa::ContextInfo) -> Vec<pb::Mention> {
    ci.mentioned_jid
        .iter()
        .filter_map(|j| relay_jid(j.clone()))
        .map(|jid| pb::Mention { jid: Some(jid) })
        .collect()
}

/// The quote a context carries. The quoted message's chat is `remote_jid` when
/// the client set it (a private reply to a group message, a reply to a status);
/// clients omit it for a reply in its own chat, so absent or empty falls back
/// to `reply_chat` (#217).
fn quote_of(ci: &wa::ContextInfo, reply_chat: &str) -> Option<pb::QuoteContext> {
    let stanza_id = ci.stanza_id.as_ref()?;
    let participant = relay_jid(ci.participant.clone().unwrap_or_default());
    let quoted_chat = ci.remote_jid.as_deref().filter(|jid| !jid.is_empty());
    Some(pb::QuoteContext {
        quoted: Some(pb::MessageKey {
            chat: relay_jid(quoted_chat.unwrap_or(reply_chat)),
            id: stanza_id.clone(),
            from_me: false,
            participant: participant.clone(),
        }),
        participant,
    })
}

/// Project a `wa::Message`'s content onto an already-addressed `InboundMessage`:
/// text, mentions, quote, reaction, edit/revoke flags, media. Wrappers are
/// peeled first (see the body), so a spoiler or ephemeral message projects its
/// content like a bare one.
///
/// Split out of `map_message` so an ECHO of a message this relay sent goes
/// through the exact same projection as one WhatsApp delivered (issue #22).
/// One code path means an edge cannot end up with two shapes for one concept.
///
/// `domain::newsletters` reuses it for a channel-history row (issue #26): a
/// channel is not E2E, so the `<plaintext>` bytes decode to the very
/// `wa::Message` a live event carries, and the same message must not project
/// two ways depending on which call returned it.
pub(crate) fn project_content(out: &mut pb::InboundMessage, msg: &wa::Message, chat: &str) {
    // Every projection reads the base message the library unwraps to (device
    // sent, ephemeral, view once, spoiler, group mentioned, audio sticker,
    // edited, ...), the same one `context_info()` looks through. Reading text
    // and media from the top level while mentions and quote come from the base
    // would project half of one message.
    let base: &wa::Message = msg.get_base_message();
    if let Some(text) = message_text(base) {
        out.text = text;
    }
    // Any message type can carry a quote and mentions (a media caption reply
    // included), so the context is read wherever the library finds it (#217).
    if let Some(ci) = msg.context_info() {
        out.mentions = mentions_of(ci);
        out.quote = quote_of(ci, chat);
    }

    out.sticker_audio = sticker_audio_descriptor(base);

    if let Some(reaction) = base.reaction_message.as_option() {
        out.reaction = reaction.text.clone().unwrap_or_default();
        out.reaction_target = reaction.key.as_option().map(wa_key_to_proto);
    }

    // An inbound edit or revoke surfaces the typed flags the proto reserves
    // (the edge otherwise reads is_edit/is_delete hard-false and never the new
    // text or target). Relay-pure: only reprojects what raw_message already
    // carries. A revoke is a top-level `protocol_message`, an edit is the one
    // inside the `edited_message` wrapper; peeling that wrapper is what puts
    // both on `base`, and reading only the top level would drop every edit.
    if let Some(pm) = base.protocol_message.as_option() {
        project_protocol_message(out, pm);
    }
    if is_secret_message_edit(base) {
        out.is_edit = true;
    }

    if let Some((descriptor, caption)) = extract_media(base) {
        out.media = Some(descriptor);
        out.caption = caption;
    }
    // `media` above is the pack's ZIP; this is the rest of it (#58).
    out.sticker_pack = base
        .sticker_pack_message
        .as_option()
        .map(sticker_packs::sticker_pack_info);
}

/// Project a message THIS relay just sent into the same `InboundMessage` shape
/// WhatsApp would have echoed, had it echoed anything (issue #22).
///
/// It does not, and that is the whole reason this exists: the relay holds ONE
/// device session per account and every consumer shares it, so a send made
/// through the socket comes from our own device and WhatsApp never sends it
/// back. Without this, each consumer of a shared relay sees only its own
/// writes, and an edge keeping a local mirror silently diverges the moment a
/// second consumer sends anything.
///
/// This is the only event on the bus that WhatsApp did not produce. It goes
/// through `project_content`, the same projection an inbound message uses, so
/// there is exactly one shape per concept.
///
/// `raw_message` is the payload as the CORE built it, not a reconstruction —
/// but not byte-identical to what the recipient decrypts either: the library's
/// send path hoists a `messageContextInfo` (device-list metadata) into the
/// encoded message when it carries none. The payload is the same; the wire
/// envelope the library adds per recipient is not. Pinned in `events.proto`
/// with the measurement.
pub fn map_sent(
    key: pb::MessageKey,
    chat: &str,
    sender: &str,
    timestamp_ms: i64,
    msg: &wa::Message,
) -> pb::InboundMessage {
    let mut out = pb::InboundMessage {
        key: Some(key),
        chat: relay_jid(chat),
        // Unset while the account has no jid yet (#122).
        sender: relay_jid(sender),
        timestamp: timestamp_ms,
        raw_message: msg.encode_to_vec(),
        ..Default::default()
    };
    project_content(&mut out, msg, chat);
    out
}

/// Project a wa `MessageKey` into the proto one (proto3 empty == lib `None`).
pub(crate) fn wa_key_to_proto(k: &wa::MessageKey) -> pb::MessageKey {
    pb::MessageKey {
        chat: relay_jid(k.remote_jid.clone().unwrap_or_default()),
        id: k.id.clone().unwrap_or_default(),
        from_me: k.from_me.unwrap_or(false),
        participant: relay_jid(k.participant.clone().unwrap_or_default()),
    }
}

/// The text of a message, whether plain `conversation` or `extended_text_message`.
fn message_text(m: &wa::Message) -> Option<String> {
    if let Some(t) = &m.conversation {
        return Some(t.clone());
    }
    m.extended_text_message
        .as_option()
        .and_then(|e| e.text.clone())
}

/// Surface an inbound edit/revoke onto the typed flags + target key. The target
/// lives in `protocol_message.key` (NOT the event's own key, which is the
/// edit/revoke stanza id), and an edit's new text lives in the nested
/// `edited_message`. Only Revoke/MessageEdit are projected; any other protocol
/// type is left untouched (it already rides in raw_message). Reading `r#type`
/// as an explicit `Some` avoids treating an absent type as Revoke (whose wire
/// value is the 0-default). waproto hands the field over typed, so there is
/// no `try_from` to do.
fn project_protocol_message(out: &mut pb::InboundMessage, pm: &wa::message::ProtocolMessage) {
    use wa::message::protocol_message::Type;
    match pm.r#type {
        Some(Type::REVOKE) => {
            out.is_delete = true;
            out.protocol_target = pm.key.as_option().map(wa_key_to_proto);
        }
        Some(Type::MESSAGE_EDIT) => {
            out.is_edit = true;
            out.protocol_target = pm.key.as_option().map(wa_key_to_proto);
            if let Some(edited) = pm.edited_message.as_option() {
                out.text = message_text(edited).unwrap_or_default();
            }
        }
        _ => {}
    }
}

/// New-style E2E edits arrive un-decrypted (`secret_encrypted_message` typed
/// MessageEdit): the new text needs the parent message's secret, which is edge
/// state. The core flags is_edit honestly and leaves the text empty.
fn is_secret_message_edit(msg: &wa::Message) -> bool {
    use wa::message::secret_encrypted_message::SecretEncType;
    msg.secret_encrypted_message
        .as_option()
        .and_then(|s| s.secret_enc_type)
        == Some(SecretEncType::MESSAGE_EDIT)
}

/// The five wa media sub-messages share identical descriptor field names but
/// no common trait (the protobuf generator emits none), so a macro projects
/// whichever one is present into a `MediaDescriptor` uniformly.
macro_rules! media_descriptor {
    ($m:expr, $kind:expr) => {
        pb::MediaDescriptor {
            direct_path: $m.direct_path.clone().unwrap_or_default(),
            media_key: $m.media_key.clone().unwrap_or_default(),
            file_enc_sha256: $m.file_enc_sha256.clone().unwrap_or_default(),
            file_sha256: $m.file_sha256.clone().unwrap_or_default(),
            file_length: $m.file_length.unwrap_or(0),
            mime_type: $m.mimetype.clone().unwrap_or_default(),
            media_type: $kind.to_wire() as i32,
        }
    };
}

/// The clip a sound sticker carries (#278), as an AUDIO descriptor. `media`
/// stays the sticker, so a client that ignores the clip still gets the image.
fn sticker_audio_descriptor(base: &wa::Message) -> Option<pb::MediaDescriptor> {
    let sticker = base.sticker_message.as_option()?;
    let Some(wa::message::sticker_message::Audio::AudioMessage(clip)) = sticker.audio.as_ref()
    else {
        return None;
    };
    Some(media_descriptor!(clip, MediaKind::Audio))
}

/// Build a `MediaDescriptor` from whichever media sub-message is present.
fn extract_media(msg: &wa::Message) -> Option<(pb::MediaDescriptor, String)> {
    let caption_of = |caption: &Option<String>| -> String { caption.clone().unwrap_or_default() };
    if let Some(m) = msg.image_message.as_option() {
        return Some((
            media_descriptor!(m, MediaKind::Image),
            caption_of(&m.caption),
        ));
    }
    if let Some(m) = msg.video_message.as_option() {
        return Some((
            media_descriptor!(m, MediaKind::Video),
            caption_of(&m.caption),
        ));
    }
    if let Some(m) = msg.audio_message.as_option() {
        return Some((media_descriptor!(m, MediaKind::Audio), String::new()));
    }
    if let Some(m) = msg.document_message.as_option() {
        return Some((
            media_descriptor!(m, MediaKind::Document),
            caption_of(&m.caption),
        ));
    }
    if let Some(m) = msg.sticker_message.as_option() {
        return Some((media_descriptor!(m, MediaKind::Sticker), String::new()));
    }
    // A pack always carries its descriptor (#58): the edge needs it to fetch the media.
    if let Some(m) = msg.sticker_pack_message.as_option() {
        return Some((sticker_packs::pack_descriptor(m), caption_of(&m.caption)));
    }
    None
}
