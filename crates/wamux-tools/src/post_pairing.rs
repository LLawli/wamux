//! What a pairing binary waits for before its first send (#108).
//!
//! Right after pairing the server closes the stream (515) and the library
//! reconnects on its own. The status cannot show that window: the daemon maps
//! `PairSuccess` to CONNECTED, and the library announces no disconnect for an
//! expected 515, so `GetAccountStatus` reads CONNECTED while a send still
//! fails with "client is not connected". The one signal that the link is back
//! is the `Connection(CONNECTED)` the new connection publishes, which is why
//! the wait reads the event stream, not the status.

use std::time::Duration;

use tokio_stream::{Stream, StreamExt};
use tonic::transport::Channel;
use wamux::proto::v1 as pb;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;

use crate::report::Report;
use crate::socket_client::ClientError;

/// How long a pairing binary waits for the post-pairing reconnect.
pub const RECONNECT_WAIT: Duration = Duration::from_secs(30);

/// Follows one account's events across the post-pairing 515: true once a
/// CONNECTED arrives after a `Paired` in the same stream.
#[derive(Debug, Default)]
pub struct PairedReconnect {
    paired: bool,
}

impl PairedReconnect {
    /// A watch for a caller that already consumed the `Paired` event itself.
    pub fn after_paired() -> Self {
        Self { paired: true }
    }

    /// Feed the next event; true when the reconnect after pairing has landed.
    pub fn observe(&mut self, envelope: &pb::EventEnvelope) -> bool {
        match &envelope.event {
            Some(pb::event_envelope::Event::Pairing(update)) => {
                if matches!(update.event, Some(pb::pairing_update::Event::Paired(_))) {
                    self.paired = true;
                }
                false
            }
            Some(pb::event_envelope::Event::Connection(change)) => {
                self.paired && change.state == pb::ConnectionState::Connected as i32
            }
            _ => false,
        }
    }
}

/// Read `events` until a CONNECTED follows a `Paired`. Errors in the stream
/// are skipped: a broadcast `Lagged` is not an end, and a tonic error is
/// followed by the end of the stream.
pub async fn wait_reconnected<S, E>(events: &mut S, within: Duration) -> Result<(), ClientError>
where
    S: Stream<Item = Result<pb::EventEnvelope, E>> + Unpin,
{
    wait_reconnected_with(events, within, PairedReconnect::default()).await
}

/// Same wait for a stream whose `Paired` was already read by the caller.
pub async fn wait_reconnected_after_paired<S, E>(
    events: &mut S,
    within: Duration,
) -> Result<(), ClientError>
where
    S: Stream<Item = Result<pb::EventEnvelope, E>> + Unpin,
{
    wait_reconnected_with(events, within, PairedReconnect::after_paired()).await
}

async fn wait_reconnected_with<S, E>(
    events: &mut S,
    within: Duration,
    mut watch: PairedReconnect,
) -> Result<(), ClientError>
where
    S: Stream<Item = Result<pb::EventEnvelope, E>> + Unpin,
{
    let follow = async {
        loop {
            match events.next().await {
                Some(Ok(envelope)) if watch.observe(&envelope) => return Ok(()),
                Some(_) => {}
                None => return Err(ClientError::EventsEnded),
            }
        }
    };
    tokio::time::timeout(within, follow)
        .await
        .unwrap_or(Err(ClientError::NotReconnected { waited: within }))
}

/// Wait for the reconnect, then send a note to `jid`. A timeout skips the
/// send with a printed note and records no check: the note was only ever
/// `accepted` (a self-chat has no receipt), so it proves nothing either way.
pub async fn send_self_after_reconnect<S, E>(
    messaging: &mut MessagingServiceClient<Channel>,
    events: &mut S,
    report: &mut Report,
    acct: pb::AccountRef,
    jid: String,
    within: Duration,
) where
    S: Stream<Item = Result<pb::EventEnvelope, E>> + Unpin,
{
    if let Err(e) = wait_reconnected(events, within).await {
        println!("self-message skipped: {e}");
        return;
    }
    let request = pb::SendTextRequest {
        account: Some(acct),
        to: Some(pb::Jid { value: jid }),
        text: "wamux: pareamento via socket + envio OK".to_string(),
        ..Default::default()
    };
    let sent = messaging.send_text(request).await;
    report.accepted_rpc("Messaging.SendText(self)", sent);
}
