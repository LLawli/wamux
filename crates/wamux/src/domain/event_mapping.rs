//! Pure mapping from whatsapp-rust `Event`s to wamux proto `EventEnvelope`
//! oneof payloads. Every inbound message carries the full protobuf in
//! `raw_message`; the typed fields are conveniences for the edge.

use std::sync::Arc;

use serde::Serialize;
use wacore::types::call::IncomingCall;
use wacore::types::events::{ConnectFailureReason, Event, Receipt, TemporaryBan};
use wacore::types::message::MessageInfo;
use wamux_types::event_enums::{
    ban_reason_of, call_action_of, chat_state_of, logout_reason_of, receipt_type_of,
    unavailable_reason_of,
};
use wamux_types::{MediaKind, relay_jid};
use whatsapp_rust::Jid;
use whatsapp_rust::buffa::Message as _;
use whatsapp_rust::waproto::whatsapp as wa;

use crate::domain::sticker_packs;
use crate::proto::v1 as pb;

/// Map an event to zero or more oneof payloads.
///
/// Zero for events we intentionally drop (raw nodes, internal notifications).
/// More than one only for `Event::Messages`, which whatsapp-rust 0.7 delivers
/// as a batch: the wire contract stays one `InboundMessage` per envelope, so
/// the batch is fanned out here rather than leaking its shape to the edge.
pub fn map_event(event: &Event) -> Vec<pb::event_envelope::Event> {
    use pb::event_envelope::Event as Pb;
    match event {
        Event::Connected(_) => one(connection(pb::ConnectionState::Connected)),
        Event::Disconnected(_) => one(connection(pb::ConnectionState::Disconnected)),
        Event::LoggedOut(l) => one(logged_out(l.reason)),
        Event::TemporaryBan(b) => one(temporary_ban(b)),

        // 0.7 sealed every payload into its own struct, so these are tuple
        // variants now instead of struct variants with a `code` field.
        Event::PairingQrCode(q) => one(pairing(pb::pairing_update::Event::QrCode(q.code.clone()))),
        Event::PairingCode(c) => one(pairing(pb::pairing_update::Event::PairCode(c.code.clone()))),
        // PairSuccess has NO push name (push names arrive later via
        // PushNameUpdate); the proto field is named for what the lib actually
        // hands over (code-review 2026-06-11: it used to masquerade as
        // push_name, empty for every personal account).
        Event::PairSuccess(p) => one(pairing(pb::pairing_update::Event::Paired(pb::PairedInfo {
            jid: Some(pb::Jid {
                value: p.id.to_string(),
            }),
            business_name: p.business_name.clone(),
            lid: Some(pb::Jid {
                value: p.lid.to_string(),
            }),
            platform: p.platform.clone(),
        }))),
        Event::PairError(p) => one(pairing(pb::pairing_update::Event::Error(
            pb::PairingError {
                message: p.error.clone(),
            },
        ))),

        // Live traffic is a batch of one; an offline drain delivers one batch
        // per durable commit. Either way the edge keeps seeing one message per
        // envelope, each with its own monotonic_seq stamped by the bridge.
        Event::Messages(batch) => batch
            .messages
            .iter()
            .map(|m| Pb::Message(map_message(&m.message, &m.info)))
            .collect(),
        Event::Receipt(r) => one(Pb::Receipt(receipt_event(r))),
        Event::UndecryptableMessage(u) => one(Pb::Undecryptable(pb::UndecryptableEvent {
            chat: lib_jid(&u.info.source.chat),
            sender: lib_jid(&u.info.source.sender),
            reason: unavailable_reason_of(u.unavailable_type) as i32,
        })),

        // The two arms fill disjoint halves of PresenceUpdate: presence answers
        // online/last_seen, a chat state answers chat_state/chat. Each leaves
        // the other half empty instead of inventing one (issue #24) — `online`
        // used to be a hardcoded true on every chat state.
        Event::Presence(p) => one(Pb::Presence(pb::PresenceUpdate {
            jid: lib_jid(&p.from),
            online: Some(!p.unavailable),
            last_seen: p.last_seen.map(|t| t.timestamp()).unwrap_or(0),
            chat_state: pb::ChatState::Unspecified as i32,
            // Unset, not empty: a real presence has no chat (#122).
            chat: None,
        })),
        // `source.chat` is what tells a group "composing" from a DM one; both
        // carry the same sender, so dropping it left the edge unable to draw the
        // indicator in the conversation it belongs to (issue #24).
        Event::ChatPresence(c) => one(Pb::Presence(pb::PresenceUpdate {
            jid: lib_jid(&c.source.sender),
            online: None,
            last_seen: 0,
            chat_state: chat_state_of(c.state, c.media) as i32,
            chat: lib_jid(&c.source.chat),
        })),

        Event::GroupUpdate(g) => one(Pb::Group(pb::GroupUpdate {
            group: lib_jid(&g.group_jid),
            kind: "group_update".to_string(),
            raw: serde_json::to_vec(g).unwrap_or_default(),
        })),
        // main retired `Event::PushNameUpdate` (upstream #1310, #30): it never
        // fired, and 0.7.0 never constructed it either, so the wire never
        // carried one. `pb::PushNameUpdate` stays in the proto so the contract
        // does not break; nothing emits it, exactly as before the bump.
        Event::ContactUpdate(c) => one(Pb::Contact(pb::ContactUpdate {
            jid: lib_jid(&c.jid),
            kind: "contact_update".to_string(),
            raw: serde_json::to_vec(c).unwrap_or_default(),
        })),

        // Backfill: only ever dispatched when the account connected with history
        // enabled (or via FetchMessageHistory). Relayed verbatim — the edge
        // decodes `raw` (a `wa.HistorySync` protobuf) itself.
        Event::HistorySync(h) => one(history_sync(h)),

        // App-state (companion-sync) chat mutations. Each lib variant is its own
        // struct sharing a `jid`/`chat_jid` + serde shape; we relay the typed
        // chat + a kind token, with the action detail in `raw`. StarUpdate names
        // its chat `chat_jid` (it points at a message, not the chat itself), so
        // it can't share the `jid`-projecting helper.
        Event::ArchiveUpdate(s) => one(Pb::AppState(app_state(
            &s.jid,
            pb::AppStateKind::Archive,
            s,
        ))),
        Event::PinUpdate(s) => one(Pb::AppState(app_state(&s.jid, pb::AppStateKind::Pin, s))),
        Event::MuteUpdate(s) => one(Pb::AppState(app_state(&s.jid, pb::AppStateKind::Mute, s))),
        Event::StarUpdate(s) => one(Pb::AppState(app_state(
            &s.chat_jid,
            pb::AppStateKind::Star,
            s,
        ))),
        Event::MarkChatAsReadUpdate(s) => one(Pb::AppState(app_state(
            &s.jid,
            pb::AppStateKind::MarkRead,
            s,
        ))),
        Event::DeleteChatUpdate(s) => one(Pb::AppState(app_state(
            &s.jid,
            pb::AppStateKind::DeleteChat,
            s,
        ))),
        // Issue #48 (upstream #1544): one list for the whole account, not one
        // chat, so it is its own event rather than an AppStateUpdate kind.
        Event::FavoritesUpdate(f) => one(Pb::FavoritesChanged(favorites_changed(f))),
        // Issue #26 (upstream #1554): a subscribed channel's tallies. It used
        // to reach the socket as a RawEvent carrying reactions only.
        Event::NewsletterLiveUpdate(u) => one(Pb::NewsletterLiveUpdate(newsletter_live_update(u))),

        // Inbound call signaling. The core relays the primitive; ring/answer
        // policy is the edge's. `call_id` is the CallAction id (the stanza id
        // lives in `raw`).
        Event::IncomingCall(c) => one(Pb::Call(map_call(c))),

        // Issue #4: the server's own verdict on an outgoing stanza, new in
        // whatsapp-rust 0.7. `SendResult` only says the library accepted the
        // message; this says whether the server did. Relayed verbatim so the
        // edge can correlate it with the send it made, and decide for itself
        // how long a missing ack is allowed to stay missing.
        Event::ServerAck(a) => one(Pb::ServerAck(pb::ServerAckEvent {
            id: a.id.clone(),
            class: a.class.clone().unwrap_or_default(),
            from: optional_lib_jid(a.from.as_ref()),
            timestamp: a.timestamp.map(|t| t.timestamp_millis()).unwrap_or(0),
            error: a.error.clone().unwrap_or_default(),
        })),

        // Issue #11: the two halves of "did this reconnect owe me a backlog".
        // `OfflineSyncPreview` is the server's own count of what it holds;
        // `OfflineSyncCompleted` is how many the drain delivered, and
        // `OfflineSyncInterrupted` (#38, upstream #1380) says the drain was cut
        // before that: not caught up, the remainder is redelivered. Typed rather
        // than left to the Raw catch-all so the comparison is part of the
        // contract instead of a JSON shape that can move underneath a consumer.
        Event::OfflineSyncPreview(p) => one(Pb::OfflineSyncPreview(pb::OfflineSyncPreview {
            total: p.total,
            messages: p.messages,
            notifications: p.notifications,
            receipts: p.receipts,
            calls: p.calls,
            statuses: p.statuses,
            app_data_changes: p.app_data_changes,
        })),
        Event::OfflineSyncCompleted(c) => one(Pb::OfflineSyncCompleted(pb::OfflineSyncCompleted {
            count: c.count,
        })),
        Event::OfflineSyncInterrupted(i) => {
            one(Pb::OfflineSyncInterrupted(pb::OfflineSyncInterrupted {
                total: i.total,
                delivered: i.delivered,
            }))
        }

        // Intentionally dropped (internal/noisy).
        Event::Notification(_) | Event::RawNode(_) => Vec::new(),

        // Forward-compat catch-all: never silently lose an event type. 0.7 also
        // made `Event` #[non_exhaustive], so this arm is now load-bearing for a
        // variant the lib adds in a minor release, not only for the ones we
        // chose not to type.
        other => one(Pb::Raw(pb::RawEvent {
            kind: variant_name(other),
            payload: serde_json::to_vec(other).unwrap_or_default(),
            note: String::new(),
        })),
    }
}

/// The single-payload case, which is every event but `Messages`.
fn one(event: pb::event_envelope::Event) -> Vec<pb::event_envelope::Event> {
    vec![event]
}

/// Backfill chunk. 0.7 keeps the payload zlib-compressed behind a lazy handle
/// (`raw_bytes()` is gone), so inflating is now fallible. A failed inflate
/// relays as the raw catch-all carrying the reason: emitting a
/// `HistorySyncEvent` with empty `raw` would tell the edge "nothing in this
/// chunk", which is a different and false statement.
fn history_sync(h: &wacore::types::events::LazyHistorySync) -> pb::event_envelope::Event {
    use pb::event_envelope::Event as Pb;
    match h.decompress() {
        Ok(raw) => Pb::HistorySync(pb::HistorySyncEvent {
            sync_type: h.sync_type(),
            chunk_order: h.chunk_order(),
            progress: h.progress(),
            session_id: h.peer_data_request_session_id().map(str::to_string),
            raw: raw.to_vec(),
        }),
        Err(e) => Pb::Raw(pb::RawEvent {
            kind: "HistorySync".to_string(),
            payload: Vec::new(),
            note: format!("history sync chunk failed to inflate: {e}"),
        }),
    }
}

/// Build an `AppStateUpdate` from any app-state lib struct. Generic over the
/// concrete update type so all six variants share the `serde_json` projection;
/// the caller passes the already-extracted chat jid (named `jid` on five of
/// them, `chat_jid` on StarUpdate).
fn app_state<T: Serialize>(chat: &Jid, kind: pb::AppStateKind, update: &T) -> pb::AppStateUpdate {
    pb::AppStateUpdate {
        // A lib Jid always renders non-empty, so this is set; the field is a
        // message only because every jid on the wire is (#122).
        chat: lib_jid(chat),
        kind: kind as i32,
        raw: serde_json::to_vec(update).unwrap_or_default(),
    }
}

/// The whole favorites list, ids verbatim and in the phone's order. `id` is
/// optional on the wire; a missing one is skipped rather than relayed as an
/// empty string a consumer would take for a chat. `raw` keeps every entry.
fn favorites_changed(update: &wacore::types::events::FavoritesUpdate) -> pb::FavoritesChanged {
    pb::FavoritesChanged {
        // An absent or empty id is dropped, never an unset Jid in the list (#122).
        chats: update
            .action
            .favorites
            .iter()
            .filter_map(|f| f.id.clone().and_then(relay_jid))
            .collect(),
        timestamp: update.timestamp.timestamp_millis(),
        from_full_sync: update.from_full_sync,
        raw: update.action.encode_to_vec(),
    }
}

/// A channel's live tallies, one entry per message the push named.
fn newsletter_live_update(
    update: &wacore::types::events::NewsletterLiveUpdate,
) -> pb::NewsletterLiveUpdate {
    pb::NewsletterLiveUpdate {
        newsletter: lib_jid(&update.newsletter_jid),
        messages: update.messages.iter().map(live_update_message).collect(),
    }
}

/// One message's counts, relayed as the server sent them. `forwards_count`
/// keeps the library's absence (no node) apart from a zero.
fn live_update_message(
    message: &wacore::types::events::NewsletterLiveUpdateMessage,
) -> pb::NewsletterLiveUpdateMessage {
    let reactions = message
        .reactions
        .iter()
        .map(|r| pb::NewsletterReactionCount {
            code: r.code.clone(),
            count: r.count,
        });
    let votes = message.votes.iter().map(|v| pb::NewsletterPollVote {
        option_hash: v.option_hash.to_vec(),
        count: v.count,
    });
    pb::NewsletterLiveUpdateMessage {
        server_id: message.server_id,
        reactions: reactions.collect(),
        votes: votes.collect(),
        forwards_count: message.forwards_count,
    }
}

/// Map an inbound call to the typed `CallEvent`. `action` is the enum, with the
/// library's wire tag in `action_raw` only when it is UNKNOWN (#126); `call_id`
/// is the CallAction id (lib note: distinct from the stanza id, which the edge
/// reads from `raw`).
fn map_call(call: &IncomingCall) -> pb::CallEvent {
    let (action, action_raw) = call_action_of(&call.action);
    pb::CallEvent {
        from: lib_jid(&call.from),
        call_id: call.action.call_id().to_string(),
        action: action as i32,
        action_raw,
        raw: serde_json::to_vec(call).unwrap_or_default(),
    }
}

fn connection(state: pb::ConnectionState) -> pb::event_envelope::Event {
    connection_with(state, None, None)
}

fn connection_with(
    state: pb::ConnectionState,
    logged_out: Option<pb::LoggedOutInfo>,
    ban: Option<pb::TemporaryBanInfo>,
) -> pb::event_envelope::Event {
    pb::event_envelope::Event::Connection(pb::ConnectionStateChanged {
        state: state as i32,
        logged_out,
        ban,
    })
}

/// The reason the server gave, as the enum plus its code when UNKNOWN (#126).
fn logged_out(reason: ConnectFailureReason) -> pb::event_envelope::Event {
    let (reason, reason_code) = logout_reason_of(reason);
    let info = pb::LoggedOutInfo {
        reason: reason as i32,
        reason_code,
    };
    connection_with(pb::ConnectionState::LoggedOut, Some(info), None)
}

/// `expire` is a duration, not a deadline, relayed in whole seconds verbatim;
/// `message` and `url` are empty when the server sent none (#126).
fn temporary_ban(ban: &TemporaryBan) -> pb::event_envelope::Event {
    let (reason, reason_code) = ban_reason_of(&ban.code);
    let info = pb::TemporaryBanInfo {
        reason: reason as i32,
        reason_code,
        expire_seconds: ban.expire.num_seconds(),
        message: ban.message.clone().unwrap_or_default(),
        url: ban.url.clone().unwrap_or_default(),
    };
    connection_with(pb::ConnectionState::Banned, None, Some(info))
}

fn receipt_event(r: &Receipt) -> pb::ReceiptEvent {
    let (receipt_type, type_raw) = receipt_type_of(&r.r#type);
    pb::ReceiptEvent {
        chat: lib_jid(&r.source.chat),
        sender: lib_jid(&r.source.sender),
        message_ids: r.message_ids.iter().map(|m| m.to_string()).collect(),
        r#type: receipt_type as i32,
        type_raw,
        timestamp: r.timestamp.timestamp_millis(),
    }
}

fn pairing(event: pb::pairing_update::Event) -> pb::event_envelope::Event {
    pb::event_envelope::Event::Pairing(pb::PairingUpdate { event: Some(event) })
}

fn map_message(msg: &Arc<wa::Message>, info: &Arc<MessageInfo>) -> pb::InboundMessage {
    let chat = info.source.chat.to_string();
    let sender = info.source.sender.to_string();
    let key = pb::MessageKey {
        chat: relay_jid(chat.clone()),
        id: info.id.to_string(),
        from_me: info.source.is_from_me,
        // The inbound participant is the sender even in a DM; kept as it was (#120).
        participant: relay_jid(sender.clone()),
    };

    let mut out = pb::InboundMessage {
        key: Some(key),
        chat: relay_jid(chat.clone()),
        sender: relay_jid(sender),
        timestamp: info.timestamp.timestamp_millis(),
        // main made this a `CompactString` (#30): a name usually short enough
        // to live inline rather than heap-allocated. The wire contract still
        // wants an owned `String`.
        push_name: info.push_name.to_string(),
        // The parser already put the stanza's other-namespace jids here
        // (sender_pn/participant_pn/participant_lid). Dropping them forced the
        // edge to poll for an identity the event itself carried (issue #1);
        // relaying them is verbatim, no lookup and no guess.
        // Unset when the stanza carried no alt jid, instead of an empty string
        // a consumer would have to know to ignore (#122).
        sender_alt: optional_lib_jid(info.source.sender_alt.as_ref()),
        recipient_alt: optional_lib_jid(info.source.recipient_alt.as_ref()),
        raw_message: msg.encode_to_vec(),
        ..Default::default()
    };

    project_content(&mut out, msg, &chat);
    out
}

/// Project a `wa::Message`'s content onto an already-addressed `InboundMessage`:
/// text, mentions, quote, reaction, edit/revoke flags, media.
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
    let chat = chat.to_string();
    if let Some(text) = &msg.conversation {
        out.text = text.clone();
    } else if let Some(ext) = msg.extended_text_message.as_option() {
        if let Some(text) = &ext.text {
            out.text = text.clone();
        }
        if let Some(ci) = ext.context_info.as_option() {
            out.mentions = ci
                .mentioned_jid
                .iter()
                // An empty mention is dropped, never `Mention { jid: None }` (#120).
                .filter_map(|j| relay_jid(j.clone()))
                .map(|jid| pb::Mention { jid: Some(jid) })
                .collect();
            if let Some(stanza_id) = &ci.stanza_id {
                let participant = relay_jid(ci.participant.clone().unwrap_or_default());
                out.quote = Some(pb::QuoteContext {
                    quoted: Some(pb::MessageKey {
                        chat: relay_jid(chat.clone()),
                        id: stanza_id.clone(),
                        from_me: false,
                        participant: participant.clone(),
                    }),
                    participant,
                });
            }
        }
    }

    if let Some(reaction) = msg.reaction_message.as_option() {
        out.reaction = reaction.text.clone().unwrap_or_default();
        out.reaction_target = reaction.key.as_option().map(wa_key_to_proto);
    }

    // An inbound edit or revoke surfaces the typed flags the proto reserves
    // (the edge otherwise reads is_edit/is_delete hard-false and never the new
    // text or target). Relay-pure: only reprojects what raw_message already
    // carries (E2E triage 2026-06-16).
    if let Some(pm) = effective_protocol_message(msg) {
        project_protocol_message(out, pm);
    }
    if is_secret_message_edit(msg) {
        out.is_edit = true;
    }

    if let Some((descriptor, caption)) = extract_media(msg) {
        out.media = Some(descriptor);
        out.caption = caption;
    }
    // Issue #58: `media` above is the pack's ZIP; this is the rest of it.
    out.sticker_pack = msg
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

/// A lib jid as the wire message, verbatim (#122).
fn lib_jid(jid: &Jid) -> Option<pb::Jid> {
    relay_jid(jid.to_string())
}

/// An optional lib jid: absent is an unset field, never an empty value (#122).
fn optional_lib_jid(jid: Option<&Jid>) -> Option<pb::Jid> {
    jid.and_then(lib_jid)
}

/// Project a wa `MessageKey` into the proto one (proto3 empty == lib `None`).
fn wa_key_to_proto(k: &wa::MessageKey) -> pb::MessageKey {
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

/// The `protocol_message` an inbound edit or revoke carries. The two arrive in
/// DIFFERENT shapes (E2E triage re-run 2026-06-16): a revoke is a top-level
/// `protocol_message`, but an edit is wrapped one level deeper, in
/// `edited_message`(FutureProofMessage)`.message.protocol_message` — exactly the
/// container the lib's `Client::edit_message` builds (client.rs:3556). Checking
/// only the top level surfaced revokes but silently dropped every edit. The
/// top-level form wins when both somehow exist.
fn effective_protocol_message(msg: &wa::Message) -> Option<&wa::message::ProtocolMessage> {
    if let Some(pm) = msg.protocol_message.as_option() {
        return Some(pm);
    }
    msg.edited_message
        .as_option()?
        .message
        .as_option()?
        .protocol_message
        .as_option()
}

/// Surface an inbound edit/revoke onto the typed flags + target key. The target
/// lives in `protocol_message.key` (NOT the event's own key, which is the
/// edit/revoke stanza id), and an edit's new text lives in the nested
/// `edited_message`. Only Revoke/MessageEdit are projected; any other protocol
/// type is left untouched (it already rides in raw_message). Reading `r#type`
/// as an explicit `Some` avoids treating an absent type as Revoke (whose wire
/// value is the 0-default). waproto 0.7 already hands the field over typed, so
/// there is no `try_from` left to do.
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
    // Issue #58: a pack used to reach the edge with no descriptor at all.
    if let Some(m) = msg.sticker_pack_message.as_option() {
        return Some((sticker_packs::pack_descriptor(m), caption_of(&m.caption)));
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
#[path = "event_mapping_action_timestamp_tests.rs"]
mod action_timestamp_tests;
#[cfg(test)]
#[path = "event_mapping_enum_tests.rs"]
mod enum_tests;
#[cfg(test)]
#[path = "event_mapping_favorites_tests.rs"]
mod favorites_tests;
#[cfg(test)]
#[path = "event_mapping_jid_tests.rs"]
mod jid_tests;
#[cfg(test)]
#[path = "event_mapping_live_update_tests.rs"]
mod live_update_tests;
#[cfg(test)]
#[path = "event_mapping_media_tests.rs"]
mod media_tests;
#[cfg(test)]
#[path = "event_mapping_tests.rs"]
mod tests;
