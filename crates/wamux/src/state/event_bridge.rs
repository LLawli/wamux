//! The `on_event` bridge: maps each whatsapp-rust event to an `EventEnvelope`,
//! stamps it with sequence + timestamp, pushes to the ring, and broadcasts it.
//! Also tracks connection state into the per-account watch channel.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use prost::Message;
use tokio::sync::{broadcast, watch};
use wacore::types::events::Event;
use whatsapp_rust::Client;

use super::event_ring::EventRing;
use crate::domain::event_mapping::map_event;
use crate::proto::v1::event_envelope::Event as WireEvent;
use crate::proto::v1::{ConnectionState, EventEnvelope};

/// Cheap-to-clone capture set for the event handler closure.
#[derive(Clone)]
pub struct EventCtx {
    pub uuid: String,
    pub events_tx: broadcast::Sender<EventEnvelope>,
    pub ring: Arc<EventRing>,
    pub state_tx: Arc<watch::Sender<ConnectionState>>,
    pub seq: Arc<AtomicI64>,
    /// Largest event (encoded bytes) still kept in the replay ring; 0 = no cap.
    pub replay_max_event_bytes: u64,
}

/// Handle one event: update connection state, then map+stamp+ring+broadcast.
pub async fn dispatch(ctx: EventCtx, event: Arc<Event>, _client: Arc<Client>) {
    update_state(&ctx, &event);

    // One lib event can fan out to several wire events: whatsapp-rust 0.7
    // delivers inbound messages in batches, and the wire contract is still one
    // message per envelope. Each gets its own monotonic_seq, so the edge keeps
    // a total order and its replay cursor keeps meaning the same thing.
    for inner in map_event(&event) {
        let envelope = EventEnvelope {
            account_uuid: ctx.uuid.clone(),
            monotonic_seq: ctx.seq.fetch_add(1, Ordering::Relaxed),
            ts_unix_ms: now_millis(),
            event: Some(inner),
        };
        // History-sync blobs are bulk, not "live" replay, and would otherwise
        // pin megabytes per account in the ring; keep them out (and any event
        // over the configured size cap). They still broadcast live.
        if replayable(&envelope, ctx.replay_max_event_bytes) {
            ctx.ring.push(envelope.clone()).await;
        }
        // Ignore "no receivers": send-only / no-subscriber accounts are normal.
        let _ = ctx.events_tx.send(envelope);
    }
}

/// Whether an event belongs in the bounded replay ring.
fn replayable(envelope: &EventEnvelope, max_bytes: u64) -> bool {
    if matches!(envelope.event, Some(WireEvent::HistorySync(_))) {
        return false;
    }
    if max_bytes > 0 && envelope.encoded_len() as u64 > max_bytes {
        return false;
    }
    true
}

fn update_state(ctx: &EventCtx, event: &Event) {
    let next = match event {
        Event::Connected(_) | Event::PairSuccess(_) => Some(ConnectionState::Connected),
        Event::Disconnected(_) => Some(ConnectionState::Disconnected),
        Event::LoggedOut(_) => Some(ConnectionState::LoggedOut),
        Event::TemporaryBan(_) => Some(ConnectionState::Banned),
        _ => None,
    };
    if let Some(state) = next {
        let _ = ctx.state_tx.send(state);
    }
}

fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::v1::{HistorySyncEvent, RawEvent};

    fn envelope(event: WireEvent) -> EventEnvelope {
        EventEnvelope {
            account_uuid: "a".to_string(),
            monotonic_seq: 0,
            ts_unix_ms: 0,
            event: Some(event),
        }
    }

    #[test]
    fn history_sync_is_never_replayable() {
        let big = HistorySyncEvent {
            raw: vec![0u8; 4096],
            ..Default::default()
        };
        assert!(!replayable(&envelope(WireEvent::HistorySync(big)), 0));
    }

    #[test]
    fn small_event_is_replayable_without_cap() {
        let raw = RawEvent {
            kind: "x".to_string(),
            payload: vec![1, 2, 3],
            note: String::new(),
        };
        assert!(replayable(&envelope(WireEvent::Raw(raw)), 0));
    }

    #[test]
    fn event_over_cap_is_dropped_from_ring() {
        let raw = RawEvent {
            kind: "x".to_string(),
            payload: vec![0u8; 1024],
            note: String::new(),
        };
        let env = envelope(WireEvent::Raw(raw));
        assert!(replayable(&env, 0)); // no cap
        assert!(!replayable(&env, 64)); // tiny cap excludes it
    }
}
