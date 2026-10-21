//! Publish an event for a message this relay sent (issue #22).
//!
//! WhatsApp echoes a message back to every device on the account EXCEPT the one
//! that sent it. The relay holds one device session per account and every
//! consumer of the socket shares it, so a send made through `MessagingService`
//! comes from our own device and nothing comes back — not live, not on the next
//! history read. The consumer that made the call has its `SendResult`; every
//! other consumer, including a second edge or a later reconnect, never learns
//! the message exists. Its receipts still arrive, naming an id nobody has.
//!
//! So the relay owes the echo that its own shape prevents. This is the one
//! event on the bus WhatsApp did not produce, and it is published exactly like
//! the ones that did: same envelope, same monotonic sequence, same ring.
//!
//! Delivered to EVERY subscriber, the caller included. Excluding the caller
//! would mean the bus knowing who called, and the core has no notion of client
//! identity; a consumer drops the duplicate on the message id, which it already
//! must do because a re-delivery has to land on the same row.

use std::sync::Arc;

use wamux_types::{Jid, MessageId};
use whatsapp_rust::waproto::whatsapp as wa;

use super::account_handle::AccountHandle;
use crate::domain::event_mapping::map_sent;
use crate::domain::messaging::sent_message_key;
use crate::proto::v1::event_envelope::Event as WireEvent;

/// Build the echo and put it on the bus. Best-effort by construction: a send
/// that reached WhatsApp is not undone by a bus with no subscribers, so nothing
/// here can fail the RPC.
///
/// The key is built here, from the id and the chat, with the same
/// `sent_message_key` the RPC answers with, so the echo and the response can
/// never disagree (#115). `sender` is this account's own phone jid, absent
/// when the library has none yet.
pub async fn publish_sent(
    handle: &Arc<AccountHandle>,
    message_id: &MessageId,
    chat: &Jid,
    sender: Option<&Jid>,
    message: &wa::Message,
) {
    let key = sent_message_key(message_id.to_string(), chat.as_lib());
    let sender = sender.map(Jid::to_string).unwrap_or_default();
    let inbound = map_sent(key, &chat.to_string(), &sender, now_millis(), message);
    // The publisher stamps the seq and timestamp and puts the echo in the
    // ring on the same terms as any other event: a consumer that reconnects
    // and replays the ring is precisely the one that missed this. It waits
    // until the echo is published so the response never outruns it.
    handle
        .publisher
        .publish_wire(WireEvent::Message(inbound))
        .await;
}

fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "send_echo_tests.rs"]
mod tests;
