//! Judging a send by delivery, never by the ack (CLAUDE.md, #47, #64).
//!
//! A `SendResult` means the library took the message and the server ack means
//! the server took the stanza; neither means a person's phone got it. A DM
//! send passes only when its fan-out reached the recipient's phone AND a
//! `delivered` receipt for its id arrives. A self-chat has neither (its
//! fan-out is all zeros), so it can only ever be `accepted`.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::sync::Notify;
use tokio_stream::{Stream, StreamExt};
use wamux::proto::v1 as pb;

use crate::report::Report;

/// The send-time half: the message id when the fan-out encrypted for at
/// least one recipient device and the phone (device 0) was not missed;
/// otherwise why not.
pub fn send_reached_phone(result: &pb::SendResult) -> Result<String, String> {
    let id: String = result
        .key
        .as_ref()
        .map(|key| key.id.clone())
        .filter(|id| !id.is_empty())
        .ok_or("the send result carries no message id")?;
    let fanout = result.recipient_fanout.as_ref().ok_or(
        "the send result has no recipient_fanout (not a DM to someone else, so nothing proves delivery)",
    )?;
    if fanout.encrypted == 0 {
        return Err(format!(
            "recipient_fanout encrypted for 0 of {} addressed devices",
            fanout.addressed
        ));
    }
    if fanout.skipped_primary {
        return Err("recipient_fanout skipped the recipient's primary phone".to_string());
    }
    Ok(id)
}

/// Receipt types that prove the message reached the recipient. `sender`,
/// `retry`, `read-self` and `server-error` say nothing of the kind.
const DELIVERY_RECEIPT_TYPES: [pb::ReceiptType; 3] = [
    pb::ReceiptType::Delivered,
    pb::ReceiptType::Read,
    pb::ReceiptType::Played,
];

/// Whether `envelope` is a receipt that proves `message_id` reached the
/// recipient: `delivered`, or the later `read` / `played`, carrying the id.
pub fn is_delivery_receipt(envelope: &pb::EventEnvelope, message_id: &str) -> bool {
    let Some(pb::event_envelope::Event::Receipt(receipt)) = &envelope.event else {
        return false;
    };
    DELIVERY_RECEIPT_TYPES.contains(&receipt.r#type())
        && receipt.message_ids.iter().any(|id| id == message_id)
}

/// Every event of one subscription, collected in the background so a binary
/// can send first and then wait for whatever the send caused.
#[derive(Clone)]
pub struct EventTap {
    seen: Arc<Mutex<Vec<pb::EventEnvelope>>>,
    arrived: Arc<Notify>,
}

impl EventTap {
    /// Drain `stream` on a background task until it ends or errors.
    pub fn spawn<S>(stream: S) -> Self
    where
        S: Stream<Item = Result<pb::EventEnvelope, tonic::Status>> + Send + Unpin + 'static,
    {
        let tap = EventTap {
            seen: Arc::default(),
            arrived: Arc::default(),
        };
        tokio::spawn(tap.clone().drain(stream));
        tap
    }

    async fn drain<S>(self, mut stream: S)
    where
        S: Stream<Item = Result<pb::EventEnvelope, tonic::Status>> + Unpin,
    {
        while let Some(Ok(envelope)) = stream.next().await {
            self.lock_seen().push(envelope);
            self.arrived.notify_waiters();
        }
    }

    /// A poisoned lock only means a panic elsewhere; the Vec is still valid.
    fn lock_seen(&self) -> MutexGuard<'_, Vec<pb::EventEnvelope>> {
        self.seen.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A copy of every event collected so far, in arrival order.
    pub fn seen(&self) -> Vec<pb::EventEnvelope> {
        self.lock_seen().clone()
    }

    /// The first event (already seen or arriving within `within`) that
    /// matches `matches`, or `None` when the window closes first.
    pub async fn wait_for(
        &self,
        within: Duration,
        matches: impl Fn(&pb::EventEnvelope) -> bool,
    ) -> Option<pb::EventEnvelope> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            // Register before looking, or an event landing in between is lost.
            let notified = self.arrived.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(found) = self.seen().into_iter().find(|e| matches(e)) {
                return Some(found);
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return self.seen().into_iter().find(|e| matches(e));
            }
        }
    }

    /// `wait_for` a delivery receipt of `message_id`.
    pub async fn delivered(&self, message_id: &str, within: Duration) -> bool {
        self.wait_for(within, |e| is_delivery_receipt(e, message_id))
            .await
            .is_some()
    }
}

/// Record a send to someone else the way the project judges one: `pass` only
/// when the fan-out reached the phone AND a `delivered` receipt for the id
/// arrives within `window`; everything else is `fail` with the reason. The tap
/// must belong to a subscription opened BEFORE the send. Returns the id when
/// it passed.
pub async fn judge_send(
    report: &mut Report,
    name: &str,
    sent: Result<pb::SendResult, tonic::Status>,
    tap: &EventTap,
    window: Duration,
) -> Option<String> {
    let (id, passed) = judge_send_keeping_id(report, name, sent, tap, window).await;
    id.filter(|_| passed)
}

/// Same judgement as `judge_send`, but also hands back the message id whenever
/// the SendResult carried one, even if the check failed: a send that went out
/// but was not delivered in time is still in the chat, and a caller that must
/// undo its side effects (revoke) needs the id. The bool is the verdict.
pub async fn judge_send_keeping_id(
    report: &mut Report,
    name: &str,
    sent: Result<pb::SendResult, tonic::Status>,
    tap: &EventTap,
    window: Duration,
) -> (Option<String>, bool) {
    let result = match sent {
        Ok(result) => result,
        Err(status) => {
            report.fail(name, format!("{}: {}", status.code(), status.message()));
            return (None, false);
        }
    };
    let sent_id: Option<String> = result
        .key
        .as_ref()
        .map(|key| key.id.clone())
        .filter(|id| !id.is_empty());
    let id = match send_reached_phone(&result) {
        Ok(id) => id,
        Err(why) => {
            report.fail(name, why);
            return (sent_id, false);
        }
    };
    let delivered = tap.delivered(&id, window).await;
    let detail = format!("id={id} delivered={delivered} (window {window:?})");
    report.verify(name, delivered, detail);
    (sent_id, delivered)
}

/// Record a send whose receipt is not the point (a reaction, an edit, a
/// revoke): `pass` when the fan-out reached the phone, `fail` otherwise.
/// Returns the id when it passed.
pub fn judge_reached(
    report: &mut Report,
    name: &str,
    sent: Result<pb::SendResult, tonic::Status>,
) -> Option<String> {
    let outcome = sent
        .map_err(|status| format!("{}: {}", status.code(), status.message()))
        .and_then(|result| send_reached_phone(&result));
    match outcome {
        Ok(id) => {
            report.pass(name, format!("id={id} reached the recipient's phone"));
            Some(id)
        }
        Err(why) => {
            report.fail(name, why);
            None
        }
    }
}
