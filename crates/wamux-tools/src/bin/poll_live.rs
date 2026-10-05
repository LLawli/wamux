//! Live validation for issue #13: create a poll, tally the votes the phone
//! casts, and cast one back.
//!
//! SAFETY: the destination is taken from `WAMUX_LIVE_DEST` (one JID); the bin
//! only ever sends there and refuses to run if it is unset, the same guard
//! `stress_live` uses, so a live test can never fan out to arbitrary numbers.
//!
//! Usage: WAMUX_LIVE_DEST=<jid> poll_live [seconds]   (default 180)
//! Env: WAMUX_REF            account external_ref (required)
//!      WAMUX_LIVE_DEST      the one chat to send to (required)
//!      WAMUX_POLL_EXPECT    the option the person votes for on the phone; the
//!                           tally must then hold the voter under exactly it
//!      WAMUX_DELIVERY_SECS  how long a send waits for its `delivered` receipt
//!
//! What to check, in this order:
//!   1. the poll shows up on the phone (delivered receipt);
//!   2. vote on the phone -> this prints a tally naming the option you chose,
//!      with `undecryptable=0`; the run compares it with WAMUX_POLL_EXPECT;
//!   3. it then votes "Sim" and, five seconds later, "Não" through the RPC.
//!      The phone must show ONE vote by this account (the second one), not two.
//!
//! The destination is spelled `@s.whatsapp.net`: the legacy server spelling
//! encrypts for nobody (issue #4) and is refused when the env is read.

use std::process::ExitCode;
use std::time::Duration;

use tonic::transport::Channel;
use whatsapp_rust::buffa::Message as _;
use whatsapp_rust::waproto::whatsapp as wa;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;
use wamux_tools::delivery::{EventTap, judge_send};
use wamux_tools::live_env::{
    account_ref_from, delivery_window_from, live_dest_from, process_env, refuse_own_number,
    socket_path_from,
};
use wamux_tools::report::Report;
use wamux_tools::socket_client::{account_ref, connect_uds, wait_connected};
use wamux_types::relay_jid;

const QUESTION: &str = "wamux #13: o voto chegou?";
const TAP_POLL: Duration = Duration::from_millis(500);

/// Everything a vote needs to be sent or tallied for this one poll.
struct PollRun {
    acct: pb::AccountRef,
    chat: String,
    creator: String,
    poll_id: String,
    secret: Vec<u8>,
    options: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let external_ref: String = account_ref_from(&process_env)?;
    let socket: String = socket_path_from(&process_env)?;
    let chat: String = live_dest_from(&process_env)?;
    let window: Duration = delivery_window_from(&process_env)?;
    let expect: Option<String> = process_env("WAMUX_POLL_EXPECT");
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(180);
    let options: Vec<String> = ["Sim", "Não", "Talvez"].map(String::from).to_vec();

    let channel = connect_uds(&socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut messaging = MessagingServiceClient::new(channel.clone());
    let mut events = EventServiceClient::new(channel);
    let acct = account_ref(&external_ref);

    let creator = wait_connected(&mut account, &acct, Duration::from_secs(30)).await?;
    println!("connected as {creator}");
    // A poll to self is the one case this validation cannot read: a note to
    // self gets no `delivered` receipt and no second party to vote (CLAUDE.md,
    // issue #4). Refuse it rather than print an inconclusive run.
    refuse_own_number(&creator, &chat)?;

    // Subscribe BEFORE creating the poll: a vote cast fast would otherwise land
    // before the stream attaches, and so would the delivered receipt.
    let stream = events
        .subscribe_events(pb::SubscribeRequest {
            selector: Some(pb::subscribe_request::Selector::Account(acct.clone())),
            replay_from_ring: 0,
        })
        .await?
        .into_inner();
    let tap = EventTap::spawn(stream);
    let mut report = Report::new();

    let created = messaging
        .send_poll(pb::SendPollRequest {
            account: Some(acct.clone()),
            to: Some(pb::Jid {
                value: chat.clone(),
            }),
            name: QUESTION.to_string(),
            options: options.clone(),
            selectable_count: 1,
        })
        .await;
    let Some(run) = judge_poll_created(
        &mut report,
        created,
        &tap,
        window,
        PollRun {
            acct,
            chat,
            creator,
            poll_id: String::new(),
            secret: Vec::new(),
            options,
        },
    )
    .await
    else {
        return Ok(report.finish());
    };
    vote_twice(&mut messaging, &mut report, &tap, window, &run).await;
    let last = collect_votes(&mut messaging, &tap, &run, secs).await;
    judge_tally(&mut report, last.as_ref(), expect.as_deref(), secs);
    Ok(report.finish())
}

/// The poll itself is a send to someone else: reached the phone and delivered.
async fn judge_poll_created(
    report: &mut Report,
    created: Result<tonic::Response<pb::SendPollResult>, tonic::Status>,
    tap: &EventTap,
    window: Duration,
    mut run: PollRun,
) -> Option<PollRun> {
    let poll = created.map(tonic::Response::into_inner);
    let as_send = poll.as_ref().map(|p| pb::SendResult {
        key: p.key.clone(),
        server_timestamp: 0,
        recipient_fanout: p.recipient_fanout,
    });
    let secret: Vec<u8> = poll
        .as_ref()
        .map(|p| p.message_secret.clone())
        .unwrap_or_default();
    let id = judge_send(
        report,
        "Messaging.SendPoll",
        as_send.map_err(|status| status.clone()),
        tap,
        window,
    )
    .await?;
    println!(
        "poll {id} created in {} (secret {} bytes); vote on the phone",
        run.chat,
        secret.len()
    );
    run.poll_id = id;
    run.secret = secret;
    Some(run)
}

/// Vote through the RPC twice: the second must REPLACE the first on the phone.
async fn vote_twice(
    messaging: &mut MessagingServiceClient<Channel>,
    report: &mut Report,
    tap: &EventTap,
    window: Duration,
    run: &PollRun,
) {
    for choice in ["Sim", "Não"] {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let sent = messaging
            .send_poll_vote(pb::SendPollVoteRequest {
                account: Some(run.acct.clone()),
                chat: Some(pb::Jid {
                    value: run.chat.clone(),
                }),
                poll_id: run.poll_id.clone(),
                poll_creator: relay_jid(run.creator.clone()),
                message_secret: run.secret.clone(),
                options: vec![choice.to_string()],
            })
            .await
            .map(tonic::Response::into_inner);
        let name = format!("Messaging.SendPollVote({choice})");
        judge_send(report, &name, sent, tap, window).await;
    }
}

/// Collect the phone's votes, oldest first (the order IS the contract), and
/// re-tally on every new one. Returns the last tally.
async fn collect_votes(
    messaging: &mut MessagingServiceClient<Channel>,
    tap: &EventTap,
    run: &PollRun,
    secs: u64,
) -> Option<pb::PollTally> {
    let mut votes: Vec<pb::PollVote> = Vec::new();
    let mut last: Option<pb::PollTally> = None;
    let mut next = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    println!("listening {secs}s for votes on poll {} ...", run.poll_id);
    while tokio::time::Instant::now() < deadline {
        let seen = tap.seen();
        for envelope in &seen[next..] {
            let Some(pb::event_envelope::Event::Message(inbound)) = &envelope.event else {
                continue;
            };
            let Some(vote) = vote_in(inbound, &run.poll_id, &run.creator) else {
                continue;
            };
            println!(
                "[vote] sender={} alt={} -> voting as {}",
                inbound.sender,
                inbound.sender_alt,
                vote.voter.as_ref().map_or("", |voter| voter.value.as_str())
            );
            votes.push(vote);
            last = aggregate(messaging, run, &votes).await.or(last);
        }
        next = seen.len();
        tokio::time::sleep(TAP_POLL).await;
    }
    last
}

async fn aggregate(
    messaging: &mut MessagingServiceClient<Channel>,
    run: &PollRun,
    votes: &[pb::PollVote],
) -> Option<pb::PollTally> {
    let tally = messaging
        .aggregate_poll_votes(pb::AggregatePollVotesRequest {
            account: Some(run.acct.clone()),
            poll_id: run.poll_id.clone(),
            poll_creator: relay_jid(run.creator.clone()),
            message_secret: run.secret.clone(),
            options: run.options.clone(),
            votes: votes.to_vec(),
        })
        .await;
    match tally {
        Ok(t) => {
            let tally = t.into_inner();
            print_tally(&tally);
            Some(tally)
        }
        Err(e) => {
            println!("AggregatePollVotes failed: {}", e.message());
            None
        }
    }
}

/// The last tally is the verdict: no vote at all, an unopened vote, or a vote
/// under another option than the one the person was asked to pick all fail.
fn judge_tally(report: &mut Report, last: Option<&pb::PollTally>, expect: Option<&str>, secs: u64) {
    const NAME: &str = "Messaging.AggregatePollVotes";
    let Some(tally) = last else {
        return report.fail(NAME, format!("no vote arrived within {secs}s"));
    };
    if tally.undecryptable != 0 {
        return report.fail(NAME, format!("undecryptable={}", tally.undecryptable));
    }
    let chosen: Vec<&str> = tally
        .results
        .iter()
        .filter(|r| !r.voters.is_empty())
        .map(|r| r.option.as_str())
        .collect();
    let ok = match expect {
        Some(option) => chosen == [option],
        None => chosen.len() == 1,
    };
    report.verify(
        NAME,
        ok,
        format!("voted options {chosen:?}, expected {expect:?}, undecryptable=0"),
    );
}

/// One vote out of an inbound message, if it belongs to this poll. The vote's
/// ciphertext only exists inside `raw_message`; the typed event fields do not
/// carry it (and the core does not decode it for us -- that is the point).
///
/// `creator` is passed so the voter can be named in the SAME namespace: the
/// vote's key is derived from the creator/voter pair, and the library only ever
/// swaps the two together, so a mixed pair (PN creator, LID voter) is never
/// tried in the combination the sender actually used. Measured on 2026-09-02:
/// feeding the stanza's `sender` verbatim gave undecryptable=3 out of 3.
fn vote_in(inbound: &pb::InboundMessage, poll_id: &str, creator: &str) -> Option<pb::PollVote> {
    let message = wa::Message::decode(&mut inbound.raw_message.as_slice()).ok()?;
    let update = message.poll_update_message.as_option()?;
    if update.poll_creation_message_key.as_option()?.id.as_deref() != Some(poll_id) {
        return None;
    }
    let vote = update.vote.as_option()?;
    Some(pb::PollVote {
        voter: relay_jid(same_namespace_as(
            creator,
            &inbound.sender,
            &inbound.sender_alt,
        )),
        enc_payload: vote.enc_payload.clone().unwrap_or_default(),
        enc_iv: vote.enc_iv.clone().unwrap_or_default(),
    })
}

/// Whichever of the sender's two forms sits in the creator's namespace. Falls
/// back to `sender` when the stanza carried no counterpart -- the core relays
/// these verbatim and never looks one up, so an absent alt stays absent.
fn same_namespace_as(creator: &str, sender: &str, sender_alt: &str) -> String {
    let is_lid = |jid: &str| jid.ends_with("@lid");
    if is_lid(creator) != is_lid(sender) && !sender_alt.is_empty() {
        return sender_alt.to_string();
    }
    sender.to_string()
}

fn print_tally(tally: &pb::PollTally) {
    for result in &tally.results {
        println!("  {} -> {:?}", result.option, result.voters);
    }
    // The whole point of the count: an empty tally and a tally whose votes
    // never opened look identical without it.
    println!("  undecryptable={}", tally.undecryptable);
}
