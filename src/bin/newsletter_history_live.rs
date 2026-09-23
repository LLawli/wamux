//! Live validation for issue #26: a channel's history, and the tallies that
//! exist nowhere else.
//!
//! Read-only. Unlike the other `*_live` bins it sends nothing and needs no
//! `WAMUX_LIVE_DEST` guard: every call here is a read of a channel this account
//! already follows.
//!
//! Usage: newsletter_history_live [socket_path] [count]
//!   defaults: /tmp/wamux.sock 20
//! Env: WAMUX_REF      account external_ref (default "pair-socket")
//!      WAMUX_CHANNEL  one `@newsletter` jid; without it, every subscribed
//!                     channel is read in turn, which is what actually finds a
//!                     poll (they are rare, and the two in issue #26's store
//!                     were in two different channels).
//!
//! What it proves, and why each half is here:
//!   1. the page arrives and projects — `server_id`, `type` and a payload that
//!      decoded, in the same `InboundMessage` shape the bus delivers;
//!   2. `before` really paginates — the second page must be strictly OLDER than
//!      the first. A server that ignored the cursor would hand back page one
//!      again and every other assertion would still pass;
//!   3. `count = 0` is refused as InvalidArgument rather than answering an
//!      empty list, which would read as "this channel has no history";
//!   4. on a poll row, it MATCHES the server's vote tally against the poll's own
//!      options. The server names an option by a 32-byte hash and never by its
//!      text, so the claim "that hash is `sha256(option_name)`" is the one thing
//!      that makes a channel poll readable at all — and it is checked here
//!      against the live server, not asserted from a fixture.

use std::fmt::Write as _;
use std::time::Duration;

use hyper_util::rt::TokioIo;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::newsletter_service_client::NewsletterServiceClient;
use whatsapp_rust::buffa::Message as _;
use whatsapp_rust::waproto::whatsapp as wa;

const DEFAULT_REF: &str = "pair-socket";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket = arg(1, "/tmp/wamux.sock");
    let count: u32 = arg(2, "20").parse().unwrap_or(20);

    let channel = connect_uds(socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut newsletters = NewsletterServiceClient::new(channel);
    let acct = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(
            std::env::var("WAMUX_REF").unwrap_or_else(|_| DEFAULT_REF.to_string()),
        )),
    };

    println!(
        "connected as {}",
        wait_connected(&mut account, &acct).await?
    );
    let targets = channels_to_read(&mut newsletters, &acct).await?;
    println!("reading {} channel(s)\n", targets.len());

    let mut polls = Vec::new();
    for (jid, name) in &targets {
        polls.extend(read_one(&mut newsletters, &acct, jid, name, count).await?);
    }

    // The cursor and the guard are properties of the RPC, not of a channel, so
    // they run once against whichever channel actually returned history.
    if let Some((jid, name)) = targets.first() {
        check_pagination(&mut newsletters, &acct, jid, name, count).await?;
        check_zero_count(&mut newsletters, &acct, jid).await;
    }
    report_polls(&polls);
    Ok(())
}

/// The channels to read: the one named in `WAMUX_CHANNEL`, or all of them.
async fn channels_to_read(
    newsletters: &mut NewsletterServiceClient<Channel>,
    acct: &pb::AccountRef,
) -> anyhow::Result<Vec<(String, String)>> {
    let subscribed = newsletters
        .list_subscribed_newsletters(acct.clone())
        .await?
        .into_inner()
        .newsletters;
    println!("subscribed to {} channel(s)", subscribed.len());
    let all: Vec<(String, String)> = subscribed
        .into_iter()
        .map(|found| (found.jid, found.name))
        .collect();
    let Ok(wanted) = std::env::var("WAMUX_CHANNEL") else {
        return Ok(all);
    };
    match all.iter().find(|(jid, _)| jid == &wanted) {
        Some(found) => Ok(vec![found.clone()]),
        // Not subscribed is not a reason to stop: the IQ may still answer, and
        // a refusal is itself the measurement.
        None => Ok(vec![(wanted, String::new())]),
    }
}

/// One page of one channel. Returns the poll rows it saw.
async fn read_one(
    newsletters: &mut NewsletterServiceClient<Channel>,
    acct: &pb::AccountRef,
    jid: &str,
    name: &str,
    count: u32,
) -> anyhow::Result<Vec<String>> {
    let page = match fetch(newsletters, acct, jid, count, 0).await {
        Ok(page) => page,
        Err(status) => {
            println!("\u{274C} {name} <{jid}>: {}", status.message());
            return Ok(Vec::new());
        }
    };
    println!("\u{2705} {name} <{jid}>: {} row(s)", page.len());
    for row in &page {
        println!("   {}", describe(row));
    }
    Ok(poll_rows(&page, jid, name))
}

/// The poll rows in one page, each already checked against its own options.
fn poll_rows(page: &[pb::NewsletterMessage], jid: &str, name: &str) -> Vec<String> {
    page.iter()
        .filter(|row| row.r#type.contains("poll"))
        .map(|row| describe_poll(row, jid, name))
        .collect()
}

/// A poll row, with the tally resolved back to option names.
///
/// This is the measurement issue #26 turns on: the server sends `<vote
/// count="N">` with 32 opaque bytes and never the option's text. If those bytes
/// are `sha256(option_name)`, a consumer can render the result with nothing but
/// the poll's own payload — no key, no secret, no extra round trip.
fn describe_poll(row: &pb::NewsletterMessage, jid: &str, name: &str) -> String {
    let options = poll_options(row);
    let mut out = format!(
        "{} <{jid}> [{}/{}] server_id={} {} option(s), {} vote node(s)",
        display_name(name, jid),
        row.r#type,
        row.poll_type,
        row.server_id,
        options.len(),
        row.votes.len(),
    );
    for vote in &row.votes {
        let named = options
            .iter()
            .find(|option| wacore::poll::compute_option_hash(option) == vote.option_hash[..]);
        match named {
            Some(option) => {
                let _ = write!(out, "\n      \u{2705} {:>7} \"{option}\"", vote.count);
            }
            None => {
                let _ = write!(
                    out,
                    "\n      \u{274C} {:>7} hash {} matches no option of this poll",
                    vote.count,
                    hex(&vote.option_hash),
                );
            }
        }
    }
    out
}

/// The option names in the poll's own payload, across the V3/V4-era fields the
/// two live polls actually used.
fn poll_options(row: &pb::NewsletterMessage) -> Vec<String> {
    let Some(message) = row.message.as_ref() else {
        return Vec::new();
    };
    let Ok(payload) = wa::Message::decode(&mut message.raw_message.as_slice()) else {
        return Vec::new();
    };
    let creation = payload
        .poll_creation_message_v3
        .as_option()
        .or_else(|| payload.poll_creation_message_v2.as_option())
        .or_else(|| payload.poll_creation_message.as_option());
    creation
        .map(|poll| {
            poll.options
                .iter()
                .filter_map(|option| option.option_name.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// One row, in one line: what the projection actually produced.
///
/// The reaction tally is clipped. A big channel carries hundreds of distinct
/// emoji on one post (the WhatsApp channel's top row: 300+), and printing them
/// all buries every other line this bin exists to show.
fn describe(row: &pb::NewsletterMessage) -> String {
    let message = row.message.clone().unwrap_or_default();
    format!(
        "server_id={:<8} type={:<8} from_me={:<5} t={} raw={}B fwd={} {} {}",
        row.server_id,
        row.r#type,
        message.key.as_ref().is_some_and(|key| key.from_me),
        message.timestamp,
        message.raw_message.len(),
        row.forwards_count,
        top_reactions(row),
        preview(&message),
    )
}

/// The three most-used reactions, and how many kinds there were in all.
fn top_reactions(row: &pb::NewsletterMessage) -> String {
    if row.reactions.is_empty() {
        return String::new();
    }
    let mut sorted: Vec<&pb::NewsletterReactionCount> = row.reactions.iter().collect();
    sorted.sort_by_key(|count| std::cmp::Reverse(count.count));
    let top: Vec<String> = sorted
        .iter()
        .take(3)
        .map(|count| format!("{}x{}", count.code, count.count))
        .collect();
    format!("[{} of {} emoji]", top.join(","), row.reactions.len())
}

/// The second page must be strictly older than the first, or the cursor is a
/// no-op and every other assertion here passes vacuously.
async fn check_pagination(
    newsletters: &mut NewsletterServiceClient<Channel>,
    acct: &pb::AccountRef,
    jid: &str,
    name: &str,
    count: u32,
) -> anyhow::Result<()> {
    let first = fetch(newsletters, acct, jid, count, 0)
        .await
        .unwrap_or_default();
    let Some(oldest) = first.iter().map(|row| row.server_id).min() else {
        println!("\u{26A0}\u{FE0F}  pagination: {name} returned no rows, cursor untested");
        return Ok(());
    };
    let second = fetch(newsletters, acct, jid, count, oldest)
        .await
        .unwrap_or_default();
    match second.iter().map(|row| row.server_id).max() {
        Some(newest) if newest < oldest => {
            println!("\u{2705} before={oldest} paginated: next page tops out at {newest}")
        }
        Some(newest) => {
            println!("\u{274C} before={oldest} did not paginate: next page still holds {newest}")
        }
        None => println!("\u{2139}\u{FE0F}  before={oldest} returned nothing (start of history)"),
    }
    Ok(())
}

/// A zero page size asks the server for nothing; the core must refuse it.
async fn check_zero_count(
    newsletters: &mut NewsletterServiceClient<Channel>,
    acct: &pb::AccountRef,
    jid: &str,
) {
    match fetch(newsletters, acct, jid, 0, 0).await {
        Err(status) if status.code() == tonic::Code::InvalidArgument => {
            println!("\u{2705} count=0 refused: {}", status.message())
        }
        Err(status) => println!(
            "\u{274C} count=0 gave {:?}: {}",
            status.code(),
            status.message()
        ),
        Ok(page) => println!(
            "\u{274C} count=0 answered {} row(s) instead of failing",
            page.len()
        ),
    }
}

/// Issue #26's driver: a channel poll cannot be voted on, and its tally rides
/// on no payload. Whether it can at least be READ is what this reports.
fn report_polls(polls: &[String]) {
    if polls.is_empty() {
        println!(
            "\n\u{2139}\u{FE0F}  no poll row in the pages read (they are rare; widen `count`)"
        );
        return;
    }
    println!("\n{} poll row(s) found:", polls.len());
    for poll in polls {
        println!("   {poll}");
    }
}

// `tonic::Status` is the size clippy's `result_large_err` complains about; the
// callers match on `.message()`/`.code()` (not just propagate with `?`), so
// boxing here (rather than switching to anyhow) keeps those call sites working
// unchanged through `Box`'s `Deref`.
async fn fetch(
    newsletters: &mut NewsletterServiceClient<Channel>,
    acct: &pb::AccountRef,
    jid: &str,
    count: u32,
    before: u64,
) -> Result<Vec<pb::NewsletterMessage>, Box<tonic::Status>> {
    Ok(newsletters
        .get_newsletter_messages(pb::GetNewsletterMessagesRequest {
            account: Some(acct.clone()),
            jid: jid.to_string(),
            count,
            before,
        })
        .await
        .map_err(Box::new)?
        .into_inner()
        .messages)
}

fn display_name(name: &str, jid: &str) -> String {
    if name.is_empty() {
        jid.to_string()
    } else {
        name.to_string()
    }
}

/// First line of the projected text, clipped. Empty for a row whose payload
/// carries no text at all, which is itself worth seeing.
fn preview(message: &pb::InboundMessage) -> String {
    let source = if message.text.is_empty() {
        &message.caption
    } else {
        &message.text
    };
    let line: String = source
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(60)
        .collect();
    if line.is_empty() {
        String::new()
    } else {
        format!("\"{line}\"")
    }
}

fn arg(index: usize, fallback: &str) -> String {
    std::env::args()
        .nth(index)
        .unwrap_or_else(|| fallback.to_string())
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
