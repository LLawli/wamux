//! Live validation for issue #22: a message sent through the relay reaching
//! consumers other than the one that sent it.
//!
//! SAFETY: the destination comes from `WAMUX_LIVE_DEST` (one JID) and the bin
//! refuses to run without it, the same guard the other live bins use. It also
//! refuses to target its own number.
//!
//! Usage: WAMUX_LIVE_DEST=<jid> send_echo_live [seconds]   (default 60)
//! Env: WAMUX_REF        the SENDING account (required)
//!      WAMUX_LIVE_DEST  the one chat to send to (required)
//!      WAMUX_DELIVERY_SECS  how long the send waits for its `delivered` receipt
//!      WAMUX_PEER_REF   a SECOND account on the same relay, if there is one.
//!                       Optional, and it is the half that reproduces the bug
//!                       exactly: it stands in for a consumer that made no call.
//!
//! What it proves, and what it cannot:
//!
//! The bug was that a send through the socket reached NO subscriber, because
//! WhatsApp does not echo a message back to the device that sent it and the
//! relay shares one device per account. So the check is a subscription that
//! made no call seeing the send appear. This bin subscribes BEFORE sending and
//! then sends, which is precisely a consumer that did not make the call --
//! the subscription and the send are independent streams on the socket.
//!
//! With `WAMUX_PEER_REF` set it also watches the OTHER account, where the
//! message arrives the ordinary way, so the two can be compared: same
//! WhatsApp message id on both sides is the property the issue asked for.

use std::process::ExitCode;
use std::time::Duration;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;
use wamux_tools::delivery::{EventTap, judge_send};
use wamux_tools::live_env::{
    PEER_REF_VAR, account_ref_from, delivery_window_from, jid_text_of, live_dest_from, process_env,
    refuse_own_number, socket_path_from,
};
use wamux_tools::report::Report;
use wamux_tools::socket_client::{account_ref, connect_uds, wait_connected};

const CONNECT_WITHIN: Duration = Duration::from_secs(30);
const TAP_POLL: Duration = Duration::from_millis(500);

/// One sighting of the sent message on a subscription: (account uuid, from_me, body).
type Sighting = (String, bool, String);

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let external_ref: String = account_ref_from(&process_env)?;
    let socket: String = socket_path_from(&process_env)?;
    let chat: String = live_dest_from(&process_env)?;
    let window: Duration = delivery_window_from(&process_env)?;
    let peer: Option<String> = process_env(PEER_REF_VAR);
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);

    let channel = connect_uds(&socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut messaging = MessagingServiceClient::new(channel.clone());
    let mut events = EventServiceClient::new(channel);
    let acct = account_ref(&external_ref);

    let own = wait_connected(&mut account, &acct, CONNECT_WITHIN).await?;
    refuse_own_number(&own, &chat)?;
    println!("sender is {own}");

    // Subscribe first, and on the ALL selector when a peer account is named, so
    // both sides of the relay are watched by one stream.
    let selector = match &peer {
        Some(_) => pb::subscribe_request::Selector::AllAccounts(pb::Empty {}),
        None => pb::subscribe_request::Selector::Account(acct.clone()),
    };
    if let Some(peer_ref) = &peer {
        // The peer must be connected or its side of the conversation never
        // arrives and the comparison would be vacuous.
        let peer_jid = wait_connected(&mut account, &account_ref(peer_ref), CONNECT_WITHIN).await?;
        println!("peer account {peer_ref} is {peer_jid}");
    }
    let stream = events
        .subscribe_events(pb::SubscribeRequest {
            selector: Some(selector),
            replay_from_ring: 0,
        })
        .await?
        .into_inner();
    let tap = EventTap::spawn(stream);
    let mut report = Report::new();

    let text = format!("wamux #22: eco de envio {}", short_stamp(&own));
    let sent = messaging
        .send_text(pb::SendTextRequest {
            account: Some(acct.clone()),
            to: Some(pb::Jid {
                value: chat.clone(),
            }),
            text,
            ..Default::default()
        })
        .await
        .map(tonic::Response::into_inner);
    let Some(sent_id) = judge_send(&mut report, "Messaging.SendText", sent, &tap, window).await
    else {
        return Ok(report.finish());
    };
    println!("sent id={sent_id} to {chat}");

    let wanted = if peer.is_some() { 2 } else { 1 };
    let seen = collect_sightings(&tap, &sent_id, secs, wanted).await;
    for (account_uuid, from_me, body) in &seen {
        println!("account {account_uuid} saw it (from_me={from_me}): {body}");
    }
    report.verify(
        "Event.InboundMessage echo of the send",
        seen.len() >= wanted,
        format!(
            "{} of {wanted} subscriber(s) saw id {sent_id} in {secs}s; none is the bug in #22",
            seen.len()
        ),
    );
    Ok(report.finish())
}

/// Every envelope naming this message id, until both sides have reported or the
/// deadline passes.
async fn collect_sightings(
    tap: &EventTap,
    sent_id: &str,
    secs: u64,
    wanted: usize,
) -> Vec<Sighting> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let seen: Vec<Sighting> = tap
            .seen()
            .iter()
            .filter_map(|envelope| sighting_of(envelope, sent_id))
            .collect();
        if seen.len() >= wanted || tokio::time::Instant::now() >= deadline {
            return seen;
        }
        tokio::time::sleep(TAP_POLL).await;
    }
}

fn sighting_of(envelope: &pb::EventEnvelope, sent_id: &str) -> Option<Sighting> {
    let Some(pb::event_envelope::Event::Message(inbound)) = &envelope.event else {
        return None;
    };
    let key = inbound.key.clone().unwrap_or_default();
    if key.id != sent_id {
        return None;
    }
    // raw_message is the point of "full fidelity": an edge decoding it must
    // find the real payload, not an empty field.
    let body = format!(
        "text={:?} raw={}B chat={}",
        inbound.text,
        inbound.raw_message.len(),
        jid_text_of(&inbound.chat)
    );
    Some((envelope.account_uuid.clone(), key.from_me, body))
}

/// A short, stable-per-run marker so two runs are distinguishable in a chat.
fn short_stamp(seed: &str) -> String {
    let digits: String = seed.chars().filter(char::is_ascii_digit).collect();
    digits.chars().rev().take(4).collect()
}
