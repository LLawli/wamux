//! Live validation for issue #22: a message sent through the relay reaching
//! consumers other than the one that sent it.
//!
//! SAFETY: the destination comes from `WAMUX_LIVE_DEST` (one JID) and the bin
//! refuses to run without it, the same guard the other live bins use. It also
//! refuses to target its own number.
//!
//! Usage: WAMUX_LIVE_DEST=<jid> send_echo_live [socket_path] [seconds]
//!   defaults: /tmp/wamux.sock 60
//! Env: WAMUX_REF        the SENDING account (default "pair-socket")
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

use std::time::Duration;

use hyper_util::rt::TokioIo;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;

const DEFAULT_REF: &str = "pair-socket";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket = arg(1, "/tmp/wamux.sock");
    let secs: u64 = arg(2, "60").parse().unwrap_or(60);
    let chat = std::env::var("WAMUX_LIVE_DEST").map_err(|_| {
        anyhow::anyhow!(
            "set WAMUX_LIVE_DEST to the destination JID (e.g. 5511999999999@s.whatsapp.net)"
        )
    })?;

    let channel = connect_uds(socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut messaging = MessagingServiceClient::new(channel.clone());
    let mut events = EventServiceClient::new(channel);
    let acct = account_ref(&std::env::var("WAMUX_REF").unwrap_or_else(|_| DEFAULT_REF.to_string()));

    let own = wait_connected(&mut account, &acct).await?;
    if same_user(&own, &chat) {
        anyhow::bail!("WAMUX_LIVE_DEST ({chat}) is this account's own number; pick another");
    }
    println!("sender is {own}");

    // Subscribe first, and on the ALL selector when a peer account is named, so
    // both sides of the relay are watched by one stream.
    let peer = std::env::var("WAMUX_PEER_REF").ok();
    let selector = match &peer {
        Some(_) => pb::subscribe_request::Selector::AllAccounts(pb::Empty {}),
        None => pb::subscribe_request::Selector::Account(acct.clone()),
    };
    if let Some(peer_ref) = &peer {
        // The peer must be connected or its side of the conversation never
        // arrives and the comparison would be vacuous.
        let peer_jid = wait_connected(&mut account, &account_ref(peer_ref)).await?;
        println!("peer account {peer_ref} is {peer_jid}");
    }
    let mut stream = events
        .subscribe_events(pb::SubscribeRequest {
            selector: Some(selector),
            replay_from_ring: 0,
        })
        .await?
        .into_inner();

    let text = format!("wamux #22: eco de envio {}", short_stamp(&own));
    let sent = messaging
        .send_text(pb::SendTextRequest {
            account: Some(acct.clone()),
            to: Some(pb::Jid {
                value: chat.clone(),
            }),
            text: text.clone(),
            ..Default::default()
        })
        .await?
        .into_inner();
    let sent_id = sent.key.map(|key| key.id).unwrap_or_default();
    println!("sent id={sent_id} to {chat}");

    let seen = collect_sightings(&mut stream, &sent_id, secs, peer.is_some()).await;
    println!();
    match seen.len() {
        0 => println!(
            "\u{274C} the send reached NO subscriber in {secs}s \u{2014} this is the bug in #22"
        ),
        _ => {
            for (account_uuid, from_me, body) in &seen {
                println!("\u{2705} account {account_uuid} saw it (from_me={from_me}): {body}");
            }
            if peer.is_some() && seen.len() < 2 {
                println!(
                    "\u{26A0}\u{FE0F}  only one side saw it; with a peer account both should, under the SAME id"
                );
            }
        }
    }
    Ok(())
}

/// Every envelope naming this message id, until both sides have reported or the
/// deadline passes.
async fn collect_sightings(
    stream: &mut tonic::Streaming<pb::EventEnvelope>,
    sent_id: &str,
    secs: u64,
    expect_two: bool,
) -> Vec<(String, bool, String)> {
    let wanted = if expect_two { 2 } else { 1 };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    let mut seen: Vec<(String, bool, String)> = Vec::new();
    while tokio::time::Instant::now() < deadline && seen.len() < wanted {
        let Ok(Ok(Some(envelope))) =
            tokio::time::timeout(Duration::from_secs(5), stream.message()).await
        else {
            continue;
        };
        let account_uuid = envelope.account_uuid.clone();
        let Some(pb::event_envelope::Event::Message(inbound)) = envelope.event else {
            continue;
        };
        let key = inbound.key.clone().unwrap_or_default();
        if key.id != sent_id {
            continue;
        }
        // raw_message is the point of "full fidelity": an edge decoding it must
        // find the real payload, not an empty field.
        let body = format!(
            "text={:?} raw={}B chat={}",
            inbound.text,
            inbound.raw_message.len(),
            inbound.chat
        );
        seen.push((account_uuid, key.from_me, body));
    }
    seen
}

fn account_ref(external: &str) -> pb::AccountRef {
    pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(external.to_string())),
    }
}

/// A short, stable-per-run marker so two runs are distinguishable in a chat.
fn short_stamp(seed: &str) -> String {
    let digits: String = seed.chars().filter(char::is_ascii_digit).collect();
    digits.chars().rev().take(4).collect()
}

fn arg(index: usize, fallback: &str) -> String {
    std::env::args()
        .nth(index)
        .unwrap_or_else(|| fallback.to_string())
}

fn user_of(jid: &str) -> String {
    jid.split('@')
        .next()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default()
        .to_string()
}

fn same_user(one: &str, other: &str) -> bool {
    user_of(one) == user_of(other)
}

async fn wait_connected(
    account: &mut AccountServiceClient<Channel>,
    acct: &pb::AccountRef,
) -> anyhow::Result<String> {
    account
        .connect_account(pb::ConnectAccountRequest {
            account: Some(acct.clone()),
            backfill_history: false,
        })
        .await?;
    for _ in 0..100 {
        let status = account.get_account_status(acct.clone()).await?.into_inner();
        if status.state == pb::ConnectionState::Connected as i32
            && let Some(jid) = status.jid
        {
            return Ok(jid.value);
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    anyhow::bail!("account did not reach Connected with a jid")
}

async fn connect_uds(path: String) -> anyhow::Result<Channel> {
    Ok(Endpoint::try_from("http://[::1]:50051")?
        .connect_with_connector(service_fn(move |_: Uri| {
            let path = path.clone();
            async move {
                Ok::<_, std::io::Error>(TokioIo::new(tokio::net::UnixStream::connect(path).await?))
            }
        }))
        .await?)
}
