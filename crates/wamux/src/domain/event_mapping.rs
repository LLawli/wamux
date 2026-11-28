//! Pure mapping from whatsapp-rust `Event`s to wamux proto `EventEnvelope`
//! oneof payloads. Every inbound message carries the full protobuf in
//! `raw_message`; the typed fields are conveniences for the edge.

mod message_projection;

use wacore::types::events::{Event, LoggedOut, Receipt, TemporaryBan};
use wacore::types::presence::PresenceStatus;
use wamux_types::event_enums::{
    ban_reason_of, chat_state_of, logout_reason_of, receipt_type_of, unavailable_reason_of,
};
use wamux_types::{relay_jid, relay_lib_jid, relay_optional_lib_jid};
use whatsapp_rust::buffa::Message as _;

use crate::domain::account_setting_update::{
    link_previews_update_of, status_privacy_update_of, unarchive_chats_update_of,
};
use crate::domain::app_state_update::{
    archive_update_of, clear_chat_update_of, delete_chat_update_of,
    delete_message_for_me_update_of, lock_chat_update_of, mark_read_update_of, mute_update_of,
    pin_update_of, star_update_of, user_status_mute_update_of,
};
use crate::domain::call_event::call_event_of;
use crate::domain::call_log_update::{call_log_history_of, call_log_update_of};
use crate::domain::connection_notice::{
    app_state_sync_failed_notice_of, client_expiration_notice_of, client_outdated_notice_of,
    connect_failure_notice_of, stream_error_notice_of, stream_replaced_notice_of,
};
use crate::domain::contact_notice::{
    about_notice_of, business_notice_of, disappearing_mode_notice_of, number_notice_of,
    picture_notice_of, profile_notice_of, sync_requested_notice_of,
};
use crate::domain::contact_update::{contact_removed_of, contact_update_of};
use crate::domain::device_notice::{device_list_notice_of, identity_notice_of};
use crate::domain::group_update::group_update_of;
use crate::domain::label_sublist_update::label_sublist_update_of;
use crate::domain::label_update::{
    label_association_update_of, label_edit_update_of, message_label_association_update_of,
};
use crate::domain::mex_notification::mex_notification_of;
use crate::domain::missed_call::missed_call_event_of;
use crate::domain::pairing_update::{
    pairing_code_error_of, pairing_code_refresh_of, pairing_qr_codes_exhausted_of,
};
use crate::domain::quick_reply_update::quick_reply_update_of;
use crate::domain::reachout_timelock_update::reachout_timelock_update_of;
use crate::domain::self_push_name_update::self_push_name_update_of;
use crate::domain::stanza_node::stanza_node_of;
use crate::domain::sticker_update::{favorite_sticker_update_of, remove_recent_sticker_update_of};
use crate::proto::v1 as pb;

use self::message_projection::map_message;
pub use self::message_projection::map_sent;
pub(crate) use self::message_projection::{project_content, wa_key_to_proto};

/// Map an event to zero or more oneof payloads.
///
/// Zero for events we intentionally drop (raw nodes, internal notifications).
/// More than one only for `Event::Messages`, which whatsapp-rust delivers
/// as a batch: the wire contract stays one `InboundMessage` per envelope, so
/// the batch is fanned out here rather than leaking its shape to the edge.
#[expect(
    clippy::too_many_lines,
    reason = "one arm per library Event variant, each delegating to its *_of function; split, the match would stop showing which events reach the wire"
)]
pub fn map_event(event: &Event) -> Vec<pb::event_envelope::Event> {
    use pb::event_envelope::Event as Pb;
    match event {
        Event::Connected(_) => one(connection(pb::ConnectionState::Connected)),
        Event::Disconnected(_) => one(connection(pb::ConnectionState::Disconnected)),
        Event::LoggedOut(l) => one(logged_out(l)),
        Event::TemporaryBan(b) => one(temporary_ban(b)),

        // Each pairing payload is its own struct in the library, hence `q.code`.
        Event::PairingQrCode(q) => one(pairing(pb::pairing_update::Event::QrCode(q.code.clone()))),
        Event::PairingCode(c) => one(pairing(pb::pairing_update::Event::PairCode(c.code.clone()))),
        // PairSuccess has NO push name (the account's own arrives later as
        // SelfPushNameUpdate); the proto field is named for what the lib
        // actually hands over.
        Event::PairSuccess(p) => one(pairing(pb::pairing_update::Event::Paired(pb::PairedInfo {
            jid: relay_lib_jid(&p.id),
            business_name: p.business_name.clone(),
            lid: relay_lib_jid(&p.lid),
            platform: p.platform.clone(),
        }))),
        Event::PairError(p) => one(pairing(pb::pairing_update::Event::Error(
            pb::PairingError {
                message: p.error.clone(),
            },
        ))),
        // Pairing lifecycle (#150).
        Event::PairingCodeRefresh(r) => one(pairing(pairing_code_refresh_of(r))),
        Event::PairingCodeError(e) => one(pairing(pairing_code_error_of(e))),
        Event::PairingQrCodesExhausted(x) => one(pairing(pairing_qr_codes_exhausted_of(x))),

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
            chat: relay_lib_jid(&u.info.source.chat),
            sender: relay_lib_jid(&u.info.source.sender),
            reason: unavailable_reason_of(u.unavailable_type) as i32,
        })),

        // The two arms fill disjoint halves of PresenceUpdate: presence answers
        // online/last_seen, a chat state answers chat_state/chat. Each leaves
        // the other half empty instead of inventing one (issue #24).
        Event::Presence(p) => one(Pb::Presence(pb::PresenceUpdate {
            jid: relay_lib_jid(&p.from),
            online: Some(!matches!(p.status, PresenceStatus::Unavailable)),
            last_seen: p.last_seen.map(|t| t.as_secs()).unwrap_or(0),
            chat_state: pb::ChatState::Unspecified as i32,
            // Unset, not empty: a real presence has no chat (#122).
            chat: None,
        })),
        // `source.chat` is what tells a group "composing" from a DM one; both
        // carry the same sender, so without it the edge cannot draw the
        // indicator in the conversation it belongs to (issue #24).
        Event::ChatPresence(c) => one(Pb::Presence(pb::PresenceUpdate {
            jid: relay_lib_jid(&c.source.sender),
            online: None,
            last_seen: 0,
            chat_state: chat_state_of(c.state) as i32,
            chat: relay_lib_jid(&c.source.chat),
        })),

        // #133: typed, one oneof case per library action.
        Event::GroupUpdate(g) => one(Pb::Group(group_update_of(g))),
        // The library retired `Event::PushNameUpdate` (upstream #1310, #30): it
        // never fired. `pb::PushNameUpdate` stays in the proto so the contract
        // does not break; nothing emits it.
        Event::ContactUpdate(c) => one(Pb::Contact(contact_update_of(c))),
        // Contact removal, the own push name and the call log (#153).
        Event::ContactRemoved(c) => one(Pb::Contact(contact_removed_of(c))),
        Event::SelfPushNameUpdated(u) => one(Pb::SelfPushName(self_push_name_update_of(u))),
        Event::CallLogSync(c) => one(Pb::CallLog(call_log_update_of(c))),
        Event::CallLogHistory(c) => one(Pb::CallLogHistory(call_log_history_of(c))),

        // Backfill: only ever dispatched when the account connected with history
        // enabled (or via FetchMessageHistory). Relayed verbatim — the edge
        // decodes `raw` (a `wa.HistorySync` protobuf) itself.
        Event::HistorySync(h) => one(history_sync(h)),

        // App-state (companion-sync) chat mutations, one concrete function per
        // library struct (#134).
        Event::ArchiveUpdate(s) => one(Pb::AppState(archive_update_of(s))),
        Event::PinUpdate(s) => one(Pb::AppState(pin_update_of(s))),
        Event::MuteUpdate(s) => one(Pb::AppState(mute_update_of(s))),
        Event::StarUpdate(s) => one(Pb::AppState(star_update_of(s))),
        Event::MarkChatAsReadUpdate(s) => one(Pb::AppState(mark_read_update_of(s))),
        Event::DeleteChatUpdate(s) => one(Pb::AppState(delete_chat_update_of(s))),
        // Chat lock and clear, message delete, status mute and stickers (#148).
        Event::LockChatUpdate(s) => one(Pb::AppState(lock_chat_update_of(s))),
        Event::ClearChatUpdate(s) => one(Pb::AppState(clear_chat_update_of(s))),
        Event::DeleteMessageForMeUpdate(s) => one(Pb::AppState(delete_message_for_me_update_of(s))),
        Event::UserStatusMuteUpdate(s) => one(Pb::AppState(user_status_mute_update_of(s))),
        Event::FavoriteStickerUpdate(s) => one(Pb::Sticker(favorite_sticker_update_of(s))),
        Event::RemoveRecentStickerUpdate(s) => one(Pb::Sticker(remove_recent_sticker_update_of(s))),
        // Labels, quick replies and account settings (#149).
        Event::LabelEditUpdate(u) => one(Pb::Label(label_edit_update_of(u))),
        Event::LabelAssociationUpdate(u) => one(Pb::Label(label_association_update_of(u))),
        Event::MessageLabelAssociationUpdate(u) => {
            one(Pb::Label(message_label_association_update_of(u)))
        }
        Event::QuickReplyUpdate(u) => one(Pb::QuickReply(quick_reply_update_of(u))),
        Event::LabelSublistUpdate(u) => one(Pb::LabelSublist(label_sublist_update_of(u))),
        Event::DisableLinkPreviewsUpdate(u) => one(Pb::AccountSetting(link_previews_update_of(u))),
        Event::StatusPrivacyUpdate(u) => one(Pb::AccountSetting(status_privacy_update_of(u))),
        Event::UnarchiveChatsSettingUpdate(u) => {
            one(Pb::AccountSetting(unarchive_chats_update_of(u)))
        }
        // Issue #48 (upstream #1544): one list for the whole account, not one
        // chat, so it is its own event rather than an AppStateUpdate kind.
        Event::FavoritesUpdate(f) => one(Pb::FavoritesChanged(favorites_changed(f))),
        // Issue #26 (upstream #1554): a subscribed channel's tallies.
        Event::NewsletterLiveUpdate(u) => one(Pb::NewsletterLiveUpdate(newsletter_live_update(u))),

        // Inbound call signaling. The core relays the primitive; ring/answer
        // policy is the edge's. Typed (#135).
        Event::IncomingCall(c) => one(Pb::Call(call_event_of(c))),
        // #151: an offer replayed from the offline queue, as a `missed` action.
        Event::MissedCall(m) => one(Pb::Call(missed_call_event_of(m))),
        // Server notifications about a contact, a device or a MEX update (#151).
        Event::PictureUpdate(u) => one(Pb::ContactNotice(picture_notice_of(u))),
        Event::UserAboutUpdate(u) => one(Pb::ContactNotice(about_notice_of(u))),
        Event::ContactUpdated(u) => one(Pb::ContactNotice(profile_notice_of(u))),
        Event::ContactNumberChanged(c) => one(Pb::ContactNotice(number_notice_of(c))),
        Event::ContactSyncRequested(r) => one(Pb::ContactNotice(sync_requested_notice_of(r))),
        Event::DisappearingModeChanged(c) => one(Pb::ContactNotice(disappearing_mode_notice_of(c))),
        Event::BusinessStatusUpdate(u) => one(Pb::ContactNotice(business_notice_of(u))),
        Event::DeviceListUpdate(u) => one(Pb::DeviceNotice(device_list_notice_of(u))),
        Event::IdentityChange(c) => one(Pb::DeviceNotice(identity_notice_of(c))),
        Event::MexNotification(n) => one(Pb::Mex(mex_notification_of(n))),
        Event::ReachoutTimelockUpdate(u) => {
            one(Pb::ReachoutTimelock(reachout_timelock_update_of(u)))
        }

        // Issue #4: the server's own verdict on an outgoing stanza.
        // `SendResult` only says the library accepted the message; this says
        // whether the server did. Relayed verbatim so the edge can correlate it
        // with the send it made, and decide for itself how long a missing ack
        // is allowed to stay missing.
        Event::ServerAck(a) => one(Pb::ServerAck(pb::ServerAckEvent {
            id: a.id.clone(),
            class: a.class.clone().unwrap_or_default(),
            from: relay_optional_lib_jid(a.from.as_ref()),
            timestamp: a.timestamp.map(|t| t.as_millis()).unwrap_or(0),
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
        // The connection's lifecycle, past its state (#150).
        Event::ConnectFailure(f) => one(Pb::ConnectionNotice(connect_failure_notice_of(f))),
        Event::ClientOutdated(o) => one(Pb::ConnectionNotice(client_outdated_notice_of(o))),
        Event::StreamError(e) => one(Pb::ConnectionNotice(stream_error_notice_of(e))),
        Event::StreamReplaced(r) => one(Pb::ConnectionNotice(stream_replaced_notice_of(r))),
        Event::AppStateSyncFailed(f) => {
            one(Pb::ConnectionNotice(app_state_sync_failed_notice_of(f)))
        }
        Event::ClientExpirationChanged(c) => {
            one(Pb::ConnectionNotice(client_expiration_notice_of(c)))
        }
        Event::OfflineSyncInterrupted(i) => {
            one(Pb::OfflineSyncInterrupted(pb::OfflineSyncInterrupted {
                total: i.total,
                delivered: i.delivered,
            }))
        }

        // Intentionally dropped (internal/noisy).
        // DirtyState too (#150, triage of #141): the library already runs the
        // resync a dirty bit asks for, so the socket has nothing to add.
        Event::Notification(_) | Event::RawNode(_) | Event::DirtyState(_) => Vec::new(),

        // Forward-compat catch-all: never silently lose an event type. `Event`
        // is #[non_exhaustive], so this arm is load-bearing for a variant the
        // lib adds in a minor release, not only for the ones we chose not to
        // type.
        other => one(Pb::Raw(raw_event_of(other))),
    }
}

/// The catch-all payload: the one place the daemon puts the library's JSON on
/// the wire (#135, guarded by scripts/check-wire-json.py). Every event wamux
/// types has its own message; only an unknown one falls back to this.
fn raw_event_of(other: &Event) -> pb::RawEvent {
    pb::RawEvent {
        kind: variant_name(other),
        payload: serde_json::to_vec(other).unwrap_or_default(),
        note: String::new(),
    }
}

/// The single-payload case, which is every event but `Messages`.
fn one(event: pb::event_envelope::Event) -> Vec<pb::event_envelope::Event> {
    vec![event]
}

/// Backfill chunk. The library keeps the payload zlib-compressed behind a lazy
/// handle, so inflating is fallible. A failed inflate
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
        timestamp: update.timestamp.as_millis(),
        from_full_sync: update.from_full_sync,
        raw: update.action.encode_to_vec(),
    }
}

/// A channel's live tallies, one entry per message the push named.
fn newsletter_live_update(
    update: &wacore::types::events::NewsletterLiveUpdate,
) -> pb::NewsletterLiveUpdate {
    pb::NewsletterLiveUpdate {
        newsletter: relay_lib_jid(&update.newsletter_jid),
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

/// The reason the server gave, as the enum plus its code when UNKNOWN (#126),
/// the server's logout copy and whether it refused the connection (#134),
/// plus the raw stanza whole (#138).
fn logged_out(logout: &LoggedOut) -> pb::event_envelope::Event {
    let (reason, reason_code) = logout_reason_of(logout.reason);
    let info = pb::LoggedOutInfo {
        reason: reason as i32,
        reason_code,
        logout_message: logout
            .logout_message
            .as_ref()
            .map(|message| pb::LogoutMessage {
                header: message.header.clone(),
                subtext: message.subtext.clone(),
                locale: message.locale.clone(),
            }),
        on_connect: logout.on_connect,
        stanza: logout.raw.as_ref().map(stanza_node_of),
    };
    connection_with(pb::ConnectionState::LoggedOut, Some(info), None)
}

/// `expire` is a duration, not a deadline, relayed in whole seconds verbatim;
/// `message` and `url` are empty when the server sent none (#126). The raw
/// stanza rides along unparsed (#138).
fn temporary_ban(ban: &TemporaryBan) -> pb::event_envelope::Event {
    let (reason, reason_code) = ban_reason_of(&ban.code);
    let info = pb::TemporaryBanInfo {
        reason: reason as i32,
        reason_code,
        expire_seconds: i64::try_from(ban.expire.as_secs()).unwrap_or(i64::MAX),
        message: ban.message.clone().unwrap_or_default(),
        url: ban.url.clone().unwrap_or_default(),
        stanza: ban.raw.as_ref().map(stanza_node_of),
    };
    connection_with(pb::ConnectionState::Banned, None, Some(info))
}

fn receipt_event(r: &Receipt) -> pb::ReceiptEvent {
    let (receipt_type, type_raw) = receipt_type_of(&r.r#type);
    pb::ReceiptEvent {
        chat: relay_lib_jid(&r.source.chat),
        sender: relay_lib_jid(&r.source.sender),
        message_ids: r.message_ids.iter().map(|m| m.to_string()).collect(),
        r#type: receipt_type as i32,
        type_raw,
        timestamp: r.timestamp.as_millis(),
    }
}

fn pairing(event: pb::pairing_update::Event) -> pb::event_envelope::Event {
    pb::event_envelope::Event::Pairing(pb::PairingUpdate { event: Some(event) })
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
#[path = "event_mapping_connection_tests.rs"]
mod connection_tests;
#[cfg(test)]
#[path = "event_mapping_echo_tests.rs"]
mod echo_tests;
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
#[path = "event_mapping_logout_tests.rs"]
mod logout_tests;
#[cfg(test)]
#[path = "event_mapping_media_tests.rs"]
mod media_tests;
#[cfg(test)]
#[path = "event_mapping_presence_tests.rs"]
mod presence_tests;
#[cfg(test)]
#[path = "event_mapping_quote_tests.rs"]
mod quote_tests;
#[cfg(test)]
#[path = "event_mapping_stanza_tests.rs"]
mod stanza_tests;
#[cfg(test)]
#[path = "event_mapping_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "event_mapping_wrapper_tests.rs"]
mod wrapper_tests;

#[cfg(test)]
#[path = "event_mapping_sound_sticker_tests.rs"]
mod sound_sticker_tests;
