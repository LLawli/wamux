//! Live validation for issue #13: create a poll, tally the votes the phone
//! casts, and cast one back.
//!
//! SAFETY: the destination is taken from `WAMUX_LIVE_DEST` (one JID); the bin
//! only ever sends there and refuses to run if it is unset, the same guard
//! `stress_live` uses, so a live test can never fan out to arbitrary numbers.
//!
//! Usage: WAMUX_LIVE_DEST=<jid> poll_live [socket_path] [seconds]
//!   defaults: /tmp/wamux.sock 180
//! Env: WAMUX_REF  account external_ref (default "pair-socket")
//!
//! What to check, in this order:
//!   1. the poll shows up on the phone;
//!   2. vote on the phone -> this prints a tally naming the option you chose,
//!      with `undecryptable=0`;
//!   3. it then votes "Sim" and, five seconds later, "Não" through the RPC.
//!      The phone must show ONE vote by this account (the second one), not two.
//!
//! Pass the destination as `@s.whatsapp.net`, never `@c.us`: the legacy
//! spelling parses as Server::Legacy and the send path then encrypts for
//! nobody (issue #4). The pinned live-send rule still spells it `@c.us`; that
//! half of the rule predates the 0.7 port. The NUMBER is the safety
//! constraint, not the server field.

use std::time::Duration;

use hyper_util::rt::TokioIo;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;
use whatsapp_rust::buffa::Message as _;
use whatsapp_rust::waproto::whatsapp as wa;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;

const DEFAULT_REF: &str = "pair-socket";
const QUESTION: &str = "wamux #13: o voto chegou?";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket = arg(1, "/tmp/wamux.sock");
    let secs: u64 = arg(2, "180").parse().unwrap_or(180);
    // Safety guard in code, not just convention: no hardcoded number, and no
    // run at all without an operator-supplied destination.
    let chat = std::env::var("WAMUX_LIVE_DEST").map_err(|_| {
        anyhow::anyhow!(
            "set WAMUX_LIVE_DEST to the destination JID (e.g. 5511999999999@s.whatsapp.net)"
        )
    })?;
    let options: Vec<String> = ["Sim", "Não", "Talvez"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    let channel = connect_uds(socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut messaging = MessagingServiceClient::new(channel.clone());
    let mut events = EventServiceClient::new(channel);
    let acct = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(
            std::env::var("WAMUX_REF").unwrap_or_else(|_| DEFAULT_REF.to_string()),
        )),
    };

    let creator = wait_connected(&mut account, &acct).await?;
    println!("connected as {creator}");
    // A poll to self is the one case this validation cannot read: a note to
    // self gets no `delivered` receipt and no second party to vote (CLAUDE.md,
    // issue #4). Refuse it rather than print an inconclusive run.
    if same_user(&creator, &chat) {
        anyhow::bail!("WAMUX_LIVE_DEST ({chat}) is this account's own number; pick the other one");
    }

    // Subscribe BEFORE creating the poll: a vote cast fast would otherwise land
    // before the stream attaches.
    let mut stream = events
        .subscribe_events(pb::SubscribeRequest {
            selector: Some(pb::subscribe_request::Selector::Account(acct.clone())),
            replay_from_ring: 0,
        })
        .await?
        .into_inner();

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
        .await?
        .into_inner();
    let poll_id = created.key.map(|k| k.id).unwrap_or_default();
    println!(
        "poll {poll_id} created in {chat} (secret {} bytes); vote on the phone",
        created.message_secret.len()
    );

    // Vote through the RPC twice: the second must REPLACE the first on the phone.
    for choice in ["Sim", "Não"] {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let sent = messaging
            .send_poll_vote(pb::SendPollVoteRequest {
                account: Some(acct.clone()),
                chat: Some(pb::Jid {
                    value: chat.clone(),
                }),
                poll_id: poll_id.clone(),
                poll_creator_jid: creator.clone(),
                message_secret: created.message_secret.clone(),
                options: vec![choice.to_string()],
            })
            .await;
        match sent {
            Ok(r) => println!(
                "✅ SendPollVote({choice}) -> {}",
                r.into_inner().key.map(|k| k.id).unwrap_or_default()
            ),
            Err(e) => println!("❌ SendPollVote({choice}): {}", e.message()),
        }
    }

    // Collect the phone's votes, oldest first (the order IS the contract), and
    // re-tally on every new one.
    let mut votes: Vec<pb::PollVote> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    println!("listening {secs}s for votes on poll {poll_id} ...");
    while tokio::time::Instant::now() < deadline {
        let Ok(Ok(Some(envelope))) =
            tokio::time::timeout(Duration::from_secs(5), stream.message()).await
        else {
            continue;
        };
        let Some(pb::event_envelope::Event::Message(inbound)) = envelope.event else {
            continue;
        };
        let Some(vote) = vote_in(&inbound, &poll_id, &creator) else {
            continue;
        };
        println!(
            "[vote] sender={} alt={} -> voting as {}",
            inbound.sender, inbound.sender_alt, vote.voter_jid
        );
        votes.push(vote);
        let tally = messaging
            .aggregate_poll_votes(pb::AggregatePollVotesRequest {
                account: Some(acct.clone()),
                poll_id: poll_id.clone(),
                poll_creator_jid: creator.clone(),
                message_secret: created.message_secret.clone(),
                options: options.clone(),
                votes: votes.clone(),
            })
            .await;
        match tally {
            Ok(t) => print_tally(&t.into_inner()),
            Err(e) => println!("❌ AggregatePollVotes: {}", e.message()),
        }
    }
    if votes.is_empty() {
        println!("❌ no vote arrived within {secs}s");
    }
    Ok(())
}

/// Same phone/lid user, ignoring the server field and any device suffix.
fn same_user(one: &str, other: &str) -> bool {
    let user = |jid: &str| {
        jid.split('@')
            .next()
            .unwrap_or_default()
            .split(':')
            .next()
            .unwrap_or_default()
            .to_string()
    };
    user(one) == user(other)
}

fn arg(index: usize, fallback: &str) -> String {
    std::env::args()
        .nth(index)
        .unwrap_or_else(|| fallback.to_string())
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
        voter_jid: same_namespace_as(creator, &inbound.sender, &inbound.sender_alt),
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

/// Connect the account and return its own jid (the poll's creator).
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
    let channel = Endpoint::try_from("http://[::1]:50051")?
        .connect_with_connector(service_fn(move |_: Uri| {
            let path = path.clone();
            async move {
                Ok::<_, std::io::Error>(TokioIo::new(tokio::net::UnixStream::connect(path).await?))
            }
        }))
        .await?;
    Ok(channel)
}
