//! Live validation for issue #24: a chat state naming the conversation it
//! happened in.
//!
//! SAFETY: both ends are accounts on this relay, named by external_ref, and the
//! bin refuses to run unless they are two different numbers. The only outward
//! effect is a group between the operator's own two accounts plus three typing
//! indicators; nothing is sent to a third party and no message is written.
//!
//! Usage: chat_state_live [seconds]   (default 45)
//! Env: WAMUX_REF        the account that TYPES (required)
//!      WAMUX_PEER_REF   the account that WATCHES (required)
//!      WAMUX_LIVE_GROUP reuse this group jid instead of creating one
//!      WAMUX_SOCKET_PATH
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

use std::process::ExitCode;
use std::time::Duration;

use tonic::transport::Channel;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::contact_service_client::ContactServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::proto::v1::group_service_client::GroupServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;
use wamux_tools::live_env::{
    PEER_REF_VAR, account_ref_from, bare_jid, jid_text_of, process_env, required_from, same_user,
    socket_path_from,
};
use wamux_tools::report::Report;
use wamux_tools::socket_client::{account_ref, connect_uds, wait_connected};

const GROUP_SUBJECT: &str = "wamux #24 chat-state";
const CONNECT_WITHIN: Duration = Duration::from_secs(30);

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    // The whole config first: nothing is opened until every variable is valid.
    let typist_ref: String = account_ref_from(&process_env)?;
    let watcher_ref: String = required_from(
        &process_env,
        PEER_REF_VAR,
        "the external_ref of the account that watches",
    )?;
    let socket: String = socket_path_from(&process_env)?;
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(45);

    let channel = connect_uds(&socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let typist = account_ref(&typist_ref);
    let watcher = account_ref(&watcher_ref);

    // The account's own jid carries its device suffix ("<user>:16@server").
    let typist_jid = bare_jid(&wait_connected(&mut account, &typist, CONNECT_WITHIN).await?);
    let watcher_jid = bare_jid(&wait_connected(&mut account, &watcher, CONNECT_WITHIN).await?);
    if same_user(&typist_jid, &watcher_jid) {
        anyhow::bail!("{typist_ref} and {watcher_ref} are the same number ({typist_jid})");
    }
    println!("typist  {typist_ref}: {typist_jid}");
    println!("watcher {watcher_ref}: {watcher_jid}");

    let mut report = Report::new();
    let mut groups = GroupServiceClient::new(channel.clone());
    let Some(group) = ensure_group(&mut groups, &mut report, &typist, &watcher_jid).await else {
        return Ok(report.finish());
    };
    println!("group: {group}");

    // A DM chat state only reaches a device that asked for this contact's
    // presence; without it the DM half would fail for a reason unrelated to #24.
    let mut contacts = ContactServiceClient::new(channel.clone());
    let subscribed = contacts
        .subscribe_presence(pb::SubscribePresenceRequest {
            account: Some(watcher.clone()),
            jid: Some(pb::Jid {
                value: typist_jid.clone(),
            }),
        })
        .await;
    report.accepted_rpc("Contact.SubscribePresence", subscribed);

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
    type_in(&mut messaging, &mut report, &typist, &group, "composing").await;
    type_in(
        &mut messaging,
        &mut report,
        &typist,
        &watcher_jid,
        "composing",
    )
    .await;
    // Recording carries the same source; it is the one whose label the mapping
    // synthesizes (Composing + Audio), so it is worth seeing on the wire too.
    type_in(&mut messaging, &mut report, &typist, &group, "recording").await;

    let seen = collect_chat_states(&mut stream, secs).await;
    judge(&mut report, &seen, &group);
    Ok(report.finish())
}

/// Raise one chat state from `account` in `chat`. `state` is the wire token the
/// proto documents (composing|recording|paused|available|unavailable).
async fn type_in(
    messaging: &mut MessagingServiceClient<Channel>,
    report: &mut Report,
    account: &pb::AccountRef,
    chat: &str,
    state: &str,
) {
    let sent = messaging
        .send_presence(pb::SendPresenceRequest {
            account: Some(account.clone()),
            chat: Some(pb::Jid {
                value: chat.to_string(),
            }),
            state: state.to_string(),
        })
        .await;
    report.accepted_rpc(&format!("Messaging.SendPresence({state} in {chat})"), sent);
}

/// Reuse `WAMUX_LIVE_GROUP` when set, otherwise create the group between the two
/// accounts. Creating it is the point on a first run: the bug needs a group both
/// numbers are in.
async fn ensure_group(
    groups: &mut GroupServiceClient<Channel>,
    report: &mut Report,
    account: &pb::AccountRef,
    participant: &str,
) -> Option<String> {
    if let Some(existing) = process_env("WAMUX_LIVE_GROUP") {
        return Some(existing);
    }
    let created = groups
        .create_group(pb::CreateGroupRequest {
            account: Some(account.clone()),
            subject: GROUP_SUBJECT.to_string(),
            participants: vec![pb::Jid {
                value: participant.to_string(),
            }],
        })
        .await;
    match created {
        Ok(resp) => {
            let jid: String = resp
                .into_inner()
                .group
                .map(|group| group.value)
                .unwrap_or_default();
            let ok = jid.ends_with("@g.us");
            report.verify("Group.CreateGroup", ok, format!("group_jid={jid}"));
            ok.then_some(jid)
        }
        Err(status) => {
            report.fail("Group.CreateGroup", status.to_string());
            None
        }
    }
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
            p.chat_state,
            jid_text_of(&p.jid),
            jid_text_of(&p.chat),
            p.online
        );
        seen.push(p);
    }
    seen
}

/// The verdict. Both halves must be present and must differ in `chat`, which is
/// the property #24 asked for; a run that saw only one half proves nothing.
fn judge(report: &mut Report, seen: &[pb::PresenceUpdate], group: &str) {
    let in_group = seen.iter().find(|p| jid_text_of(&p.chat) == group);
    let in_dm = seen
        .iter()
        .find(|p| !jid_text_of(&p.chat).is_empty() && !jid_text_of(&p.chat).contains("@g.us"));
    match in_group {
        Some(g) => report.pass(
            "group chat state names the group",
            format!("chat={}", jid_text_of(&g.chat)),
        ),
        None => report.fail(
            "group chat state names the group",
            format!("none of {} chat states carried chat={group}", seen.len()),
        ),
    }
    match in_dm {
        Some(d) => report.pass(
            "direct chat state names the contact",
            format!("chat={}", jid_text_of(&d.chat)),
        ),
        None => report.fail(
            "direct chat state names the contact",
            format!("the DM half is missing among {} chat states", seen.len()),
        ),
    }
    let lit = seen.iter().filter(|p| p.online.is_some()).count();
    report.verify(
        "online is absent on every chat state (no hardcoded true)",
        lit == 0 && !seen.is_empty(),
        format!(
            "{lit} of {} chat states carry an `online` value",
            seen.len()
        ),
    );
}
