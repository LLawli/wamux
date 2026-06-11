//! Pure mapping from whatsapp-rust `Event`s to wamux proto `EventEnvelope`
//! oneof payloads. Every inbound message carries the full protobuf in
//! `raw_message`; the typed fields are conveniences for the edge.

use std::sync::Arc;

use prost014::Message as _;
use wacore::types::events::Event;
use wacore::types::message::MessageInfo;
use wacore::types::presence::{ChatPresence, ChatPresenceMedia, ReceiptType};
use whatsapp_rust::waproto::whatsapp as wa;

use crate::proto::v1 as pb;

/// Map an event to a oneof payload, or `None` for events we intentionally drop
/// (raw nodes, internal notifications).
pub fn map_event(event: &Event) -> Option<pb::event_envelope::Event> {
    use pb::event_envelope::Event as Pb;
    match event {
        Event::Connected(_) => Some(connection(pb::ConnectionState::Connected, "")),
        Event::Disconnected(_) => Some(connection(pb::ConnectionState::Disconnected, "")),
        Event::LoggedOut(l) => Some(connection(
            pb::ConnectionState::LoggedOut,
            &format!("{:?}", l.reason),
        )),
        Event::TemporaryBan(b) => Some(connection(pb::ConnectionState::Banned, &format!("{b:?}"))),

        Event::PairingQrCode { code, .. } => {
            Some(pairing(pb::pairing_update::Event::QrCode(code.clone())))
        }
        Event::PairingCode { code, .. } => {
            Some(pairing(pb::pairing_update::Event::PairCode(code.clone())))
        }
        // PairSuccess has NO push name (push names arrive later via
        // PushNameUpdate); the proto field is named for what the lib actually
        // hands over (code-review 2026-06-11: it used to masquerade as
        // push_name, empty for every personal account).
        Event::PairSuccess(p) => Some(pairing(pb::pairing_update::Event::Paired(pb::PairedInfo {
            jid: Some(pb::Jid {
                value: p.id.to_string(),
            }),
            business_name: p.business_name.clone(),
            lid: Some(pb::Jid {
                value: p.lid.to_string(),
            }),
            platform: p.platform.clone(),
        }))),
        Event::PairError(p) => Some(pairing(pb::pairing_update::Event::Error(
            pb::PairingError {
                message: p.error.clone(),
            },
        ))),

        Event::Message(msg, info) => Some(Pb::Message(map_message(msg, info))),
        Event::Receipt(r) => Some(Pb::Receipt(pb::ReceiptEvent {
            chat: r.source.chat.to_string(),
            sender: r.source.sender.to_string(),
            message_ids: r.message_ids.iter().map(|m| m.to_string()).collect(),
            r#type: receipt_type_label(&r.r#type),
            timestamp: r.timestamp.timestamp_millis(),
        })),
        Event::UndecryptableMessage(u) => Some(Pb::Undecryptable(pb::UndecryptableEvent {
            chat: u.info.source.chat.to_string(),
            sender: u.info.source.sender.to_string(),
            reason: format!("{:?}", u.unavailable_type),
        })),

        Event::Presence(p) => Some(Pb::Presence(pb::PresenceUpdate {
            jid: p.from.to_string(),
            online: !p.unavailable,
            last_seen: p.last_seen.map(|t| t.timestamp()).unwrap_or(0),
            chat_state: String::new(),
        })),
        Event::ChatPresence(c) => Some(Pb::Presence(pb::PresenceUpdate {
            jid: c.source.sender.to_string(),
            online: true,
            last_seen: 0,
            chat_state: chat_state_label(c.state, c.media).to_string(),
        })),

        Event::GroupUpdate(g) => Some(Pb::Group(pb::GroupUpdate {
            group_jid: g.group_jid.to_string(),
            kind: "group_update".to_string(),
            raw: serde_json::to_vec(g).unwrap_or_default(),
        })),
        Event::PushNameUpdate(p) => Some(Pb::PushName(pb::PushNameUpdate {
            jid: p.jid.to_string(),
            push_name: p.new_push_name.clone(),
        })),
        Event::ContactUpdate(c) => Some(Pb::Contact(pb::ContactUpdate {
            jid: c.jid.to_string(),
            kind: "contact_update".to_string(),
            raw: serde_json::to_vec(c).unwrap_or_default(),
        })),

        // Backfill: only ever dispatched when the account connected with history
        // enabled (or via FetchMessageHistory). Relayed verbatim — the edge
        // decodes `raw` (a `wa.HistorySync` protobuf) itself.
        Event::HistorySync(h) => Some(Pb::HistorySync(pb::HistorySyncEvent {
            sync_type: h.sync_type(),
            chunk_order: h.chunk_order(),
            progress: h.progress(),
            session_id: h.peer_data_request_session_id().map(str::to_string),
            raw: h.raw_bytes().to_vec(),
        })),

        // Intentionally dropped (internal/noisy).
        Event::Notification(_) | Event::RawNode(_) => None,

        // Forward-compat catch-all: never silently lose an event type.
        other => Some(Pb::Raw(pb::RawEvent {
            kind: variant_name(other),
            payload: serde_json::to_vec(other).unwrap_or_default(),
            note: String::new(),
        })),
    }
}

/// Receipt types relay as the lowercase tokens `ReceiptEvent.type` documents
/// (code-review 2026-06-11: the old `{:?}` Debug casing — "Read" — broke any
/// edge written against the proto contract). `Other` already carries the raw
/// stanza attribute, so it relays verbatim.
fn receipt_type_label(receipt: &ReceiptType) -> String {
    match receipt {
        ReceiptType::Delivered => "delivered".to_string(),
        ReceiptType::Sender => "sender".to_string(),
        ReceiptType::Retry => "retry".to_string(),
        ReceiptType::EncRekeyRetry => "enc_rekey_retry".to_string(),
        ReceiptType::Read => "read".to_string(),
        ReceiptType::ReadSelf => "read-self".to_string(),
        ReceiptType::Played => "played".to_string(),
        ReceiptType::PlayedSelf => "played-self".to_string(),
        ReceiptType::ServerError => "server-error".to_string(),
        ReceiptType::Inactive => "inactive".to_string(),
        ReceiptType::PeerMsg => "peer_msg".to_string(),
        ReceiptType::HistorySync => "hist_sync".to_string(),
        ReceiptType::Other(raw) => raw.clone(),
    }
}

/// The lib models "recording" as Composing with media=Audio; the wire contract
/// (`composing|recording|paused`, the same tokens SendPresence accepts back)
/// flattens that pair into one token.
fn chat_state_label(state: ChatPresence, media: ChatPresenceMedia) -> &'static str {
    match (state, media) {
        (ChatPresence::Composing, ChatPresenceMedia::Audio) => "recording",
        (ChatPresence::Composing, ChatPresenceMedia::Text) => "composing",
        (ChatPresence::Paused, _) => "paused",
    }
}

fn connection(state: pb::ConnectionState, detail: &str) -> pb::event_envelope::Event {
    pb::event_envelope::Event::Connection(pb::ConnectionStateChanged {
        state: state as i32,
        detail: detail.to_string(),
    })
}

fn pairing(event: pb::pairing_update::Event) -> pb::event_envelope::Event {
    pb::event_envelope::Event::Pairing(pb::PairingUpdate { event: Some(event) })
}

fn map_message(msg: &Arc<wa::Message>, info: &Arc<MessageInfo>) -> pb::InboundMessage {
    let chat = info.source.chat.to_string();
    let sender = info.source.sender.to_string();
    let key = pb::MessageKey {
        remote_jid: chat.clone(),
        id: info.id.to_string(),
        from_me: info.source.is_from_me,
        participant: sender.clone(),
    };

    let mut out = pb::InboundMessage {
        key: Some(key),
        chat: chat.clone(),
        sender,
        timestamp: info.timestamp.timestamp_millis(),
        push_name: info.push_name.clone(),
        raw_message: msg.encode_to_vec(),
        ..Default::default()
    };

    if let Some(text) = &msg.conversation {
        out.text = text.clone();
    } else if let Some(ext) = &msg.extended_text_message {
        if let Some(text) = &ext.text {
            out.text = text.clone();
        }
        if let Some(ci) = &ext.context_info {
            out.mentions = ci
                .mentioned_jid
                .iter()
                .map(|j| pb::Mention { jid: j.clone() })
                .collect();
            if let Some(stanza_id) = &ci.stanza_id {
                let participant = ci.participant.clone().unwrap_or_default();
                out.quote = Some(pb::QuoteContext {
                    quoted: Some(pb::MessageKey {
                        remote_jid: chat.clone(),
                        id: stanza_id.clone(),
                        from_me: false,
                        participant: participant.clone(),
                    }),
                    participant,
                });
            }
        }
    }

    if let Some(reaction) = &msg.reaction_message {
        out.reaction = reaction.text.clone().unwrap_or_default();
        if let Some(k) = &reaction.key {
            out.reaction_target = Some(pb::MessageKey {
                remote_jid: k.remote_jid.clone().unwrap_or_default(),
                id: k.id.clone().unwrap_or_default(),
                from_me: k.from_me.unwrap_or(false),
                participant: k.participant.clone().unwrap_or_default(),
            });
        }
    }

    if let Some((descriptor, caption)) = extract_media(msg) {
        out.media = Some(descriptor);
        out.caption = caption;
    }

    out
}

/// The five wa media sub-messages share identical descriptor field names but
/// no common trait (prost-generated structs), so a macro projects whichever
/// one is present into a `MediaDescriptor` uniformly.
macro_rules! media_descriptor {
    ($m:expr, $kind:literal) => {
        pb::MediaDescriptor {
            direct_path: $m.direct_path.clone().unwrap_or_default(),
            media_key: $m.media_key.clone().unwrap_or_default(),
            file_enc_sha256: $m.file_enc_sha256.clone().unwrap_or_default(),
            file_sha256: $m.file_sha256.clone().unwrap_or_default(),
            file_length: $m.file_length.unwrap_or(0),
            mime_type: $m.mimetype.clone().unwrap_or_default(),
            media_type: $kind.to_string(),
        }
    };
}

/// Build a `MediaDescriptor` from whichever media sub-message is present.
fn extract_media(msg: &wa::Message) -> Option<(pb::MediaDescriptor, String)> {
    let caption_of = |caption: &Option<String>| -> String { caption.clone().unwrap_or_default() };
    if let Some(m) = &msg.image_message {
        return Some((media_descriptor!(m, "image"), caption_of(&m.caption)));
    }
    if let Some(m) = &msg.video_message {
        return Some((media_descriptor!(m, "video"), caption_of(&m.caption)));
    }
    if let Some(m) = &msg.audio_message {
        return Some((media_descriptor!(m, "audio"), String::new()));
    }
    if let Some(m) = &msg.document_message {
        return Some((media_descriptor!(m, "document"), caption_of(&m.caption)));
    }
    if let Some(m) = &msg.sticker_message {
        return Some((media_descriptor!(m, "sticker"), String::new()));
    }
    None
}

/// Best-effort variant name for the catch-all `RawEvent.kind`.
fn variant_name(event: &Event) -> String {
    let debug = format!("{event:?}");
    debug
        .split(['(', '{', ' '])
        .next()
        .unwrap_or("Event")
        .to_string()
}

// Tests live in sibling files to keep each one under the 500-line rule.
#[cfg(test)]
#[path = "event_mapping_media_tests.rs"]
mod media_tests;
#[cfg(test)]
#[path = "event_mapping_tests.rs"]
mod tests;
