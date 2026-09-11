//! Live validation for issue #24: a chat state naming the conversation it
//! happened in.
//!
//! SAFETY: both ends are accounts on this relay, named by external_ref, and the
//! bin refuses to run unless they are two different numbers. The only outward
//! effect is a group between the operator's own two accounts plus three typing
//! indicators; nothing is sent to a third party and no message is written.
//!
//! Usage: chat_state_live [socket_path] [seconds]
//!   defaults: $HOME/.local/state/wamux/wamux.sock 45
//! Env: WAMUX_REF        the account that TYPES (default "pessoal")
//!      WAMUX_PEER_REF   the account that WATCHES (default "trabalho")
//!      WAMUX_LIVE_GROUP reuse this group jid instead of creating one
//!
//! What it proves, and what it cannot:
//!
//! The bug was that `Event::ChatPresence` reached the edge carrying only its
//! sender, so a `composing` raised in a group was byte-identical to one raised
//! in the direct chat with the same person. The check therefore needs BOTH
//! halves in one run: the typist raises a chat state in the group and then in
//! the DM, and the watcher must report two events that differ in `chat`. One
//! half alone proves nothing -- a build that hardcoded the group jid would pass
//! the group half.
//!
//! It also asserts `online` is ABSENT on a chat state (issue #24): it used to
//! be a hardcoded true, which a consumer could light an "online" dot from.
//!
//! What it cannot do is prove the indicator renders. That is the edge's job;
//! this only shows the bytes now carry the conversation.
//!
//! Measured 2026-09-11 against the two production accounts: the group half
//! names the group jid, and the DM half names the sender's `@lid` (the same
//! value as `jid`, because that is the namespace the stanza used). The core
//! relays whichever namespace arrived; an edge filing chats by number resolves
//! it with ContactService.ResolveLidPn.

use std::time::Duration;

use hyper_util::rt::TokioIo;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::contact_service_client::ContactServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::proto::v1::group_service_client::GroupServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;

const DEFAULT_REF: &str = "pessoal";
const DEFAULT_PEER_REF: &str = "trabalho";
const GROUP_SUBJECT: &str = "wamux #24 chat-state";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket = arg(1, &default_socket());
    let secs: u64 = arg(2, "45").parse().unwrap_or(45);
    let typist_ref = env_or("WAMUX_REF", DEFAULT_REF);
    let watcher_ref = env_or("WAMUX_PEER_REF", DEFAULT_PEER_REF);

    let channel = connect_uds(socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let typist = account_ref(&typist_ref);
    let watcher = account_ref(&watcher_ref);

    // The account's own jid carries its device suffix ("<user>:16@server"). A
    // recipient is a user, not a device: the server answers 400 bad-request to a
    // group whose participant names one.
    let typist_jid = bare_jid(&wait_connected(&mut account, &typist).await?);
    let watcher_jid = bare_jid(&wait_connected(&mut account, &watcher).await?);
    if same_user(&typist_jid, &watcher_jid) {
        anyhow::bail!("{typist_ref} and {watcher_ref} are the same number ({typist_jid})");
    }
    println!("typist  {typist_ref}: {typist_jid}");
    println!("watcher {watcher_ref}: {watcher_jid}");

    let mut groups = GroupServiceClient::new(channel.clone());
    let group = ensure_group(&mut groups, &typist, &watcher_jid).await?;
    println!("group: {group}");

    // A DM chat state only reaches a device that asked for this contact's
    // presence; without it the DM half would fail for a reason unrelated to #24.
    let mut contacts = ContactServiceClient::new(channel.clone());
    contacts
        .subscribe_presence(pb::SubscribePresenceRequest {
            account: Some(watcher.clone()),
            jid: typist_jid.clone(),
        })
        .await?;

    // Subscribe before typing: the events are only observable while the stream
    // is open, and a chat state is not replayed.
    let mut events = EventServiceClient::new(channel.clone());
    let mut stream = events
        .subscribe_events(pb::SubscribeRequest {
            selector: Some(pb::subscribe_request::Selector::Account(watcher.clone())),
            replay_from_ring: 0,
        })
        .await?
        .into_inner();

    let mut messaging = MessagingServiceClient::new(channel);
    type_in(&mut messaging, &typist, &group, "composing").await?;
    type_in(&mut messaging, &typist, &watcher_jid, "composing").await?;
    // Recording carries the same source; it is the one whose label the mapping
    // synthesizes (Composing + Audio), so it is worth seeing on the wire too.
    type_in(&mut messaging, &typist, &group, "recording").await?;

    let seen = collect_chat_states(&mut stream, secs).await;
    report(&seen, &group)
}

/// Raise one chat state from `account` in `chat`. `state` is the wire token the
/// proto documents (composing|recording|paused|available|unavailable).
async fn type_in(
    messaging: &mut MessagingServiceClient<Channel>,
    account: &pb::AccountRef,
    chat: &str,
    state: &str,
) -> anyhow::Result<()> {
    messaging
        .send_presence(pb::SendPresenceRequest {
            account: Some(account.clone()),
            chat: Some(pb::Jid {
                value: chat.to_string(),
            }),
            state: state.to_string(),
        })
        .await?;
    println!("typed {state} in {chat}");
    Ok(())
}

/// Reuse `WAMUX_LIVE_GROUP` when set, otherwise create the group between the two
/// accounts. Creating it is the point on a first run: the bug needs a group both
/// numbers are in.
async fn ensure_group(
    groups: &mut GroupServiceClient<Channel>,
    account: &pb::AccountRef,
    participant: &str,
) -> anyhow::Result<String> {
    if let Ok(existing) = std::env::var("WAMUX_LIVE_GROUP") {
        return Ok(existing);
    }
    let created = groups
        .create_group(pb::CreateGroupRequest {
            account: Some(account.clone()),
            subject: GROUP_SUBJECT.to_string(),
            participants: vec![participant.to_string()],
        })
        .await?
        .into_inner();
    Ok(created.group_jid)
}

/// Every chat state the watcher sees, until the deadline. Real presence
/// (`chat_state` empty) is dropped: it is a different event and would only
/// muddy the comparison.
async fn collect_chat_states(
    stream: &mut tonic::Streaming<pb::EventEnvelope>,
    secs: u64,
) -> Vec<pb::PresenceUpdate> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    let mut seen: Vec<pb::PresenceUpdate> = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Ok(Ok(Some(envelope))) =
            tokio::time::timeout(Duration::from_secs(5), stream.message()).await
        else {
            continue;
        };
        let Some(pb::event_envelope::Event::Presence(p)) = envelope.event else {
            continue;
        };
        if p.chat_state.is_empty() {
            continue;
        }
        println!(
            "[recv] chat_state={} jid={} chat={:?} online={:?}",
            p.chat_state, p.jid, p.chat, p.online
        );
        seen.push(p);
    }
    seen
}

/// The verdict. Both halves must be present and must differ in `chat`, which is
/// the property #24 asked for; a run that saw only one half proves nothing.
fn report(seen: &[pb::PresenceUpdate], group: &str) -> anyhow::Result<()> {
    println!();
    let in_group = seen.iter().find(|p| p.chat == group);
    let in_dm = seen
        .iter()
        .find(|p| !p.chat.is_empty() && !p.chat.contains("@g.us"));
    match (in_group, in_dm) {
        (Some(g), Some(d)) => {
            println!("\u{2705} group chat state names the group: chat={}", g.chat);
            println!(
                "\u{2705} direct chat state names the contact: chat={}",
                d.chat
            );
            println!("\u{2705} the two are distinguishable, which is issue #24");
        }
        (Some(g), None) => anyhow::bail!(
            "only the group half arrived (chat={}); the DM half is missing, so nothing is proven",
            g.chat
        ),
        (None, Some(d)) => anyhow::bail!(
            "only the direct half arrived (chat={}); the group chat state never showed up",
            d.chat
        ),
        (None, None) if seen.is_empty() => anyhow::bail!("no chat state arrived at all"),
        (None, None) => anyhow::bail!("{} chat states arrived, none carrying a chat", seen.len()),
    }
    let lit: Vec<&pb::PresenceUpdate> = seen.iter().filter(|p| p.online.is_some()).collect();
    match lit.is_empty() {
        true => println!("\u{2705} online is absent on every chat state (no hardcoded true)"),
        false => anyhow::bail!("{} chat states still carry an `online` value", lit.len()),
    }
    Ok(())
}

fn account_ref(external: &str) -> pb::AccountRef {
    pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(external.to_string())),
    }
}

fn default_socket() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    format!("{home}/.local/state/wamux/wamux.sock")
}

fn env_or(key: &str, fallback: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| fallback.to_string())
}

fn arg(index: usize, fallback: &str) -> String {
    std::env::args()
        .nth(index)
        .unwrap_or_else(|| fallback.to_string())
}

/// Drop the device suffix: "5511999999999:16@s.whatsapp.net" -> the user jid.
fn bare_jid(jid: &str) -> String {
    let Some((user, server)) = jid.split_once('@') else {
        return jid.to_string();
    };
    format!("{}@{server}", user.split(':').next().unwrap_or(user))
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
