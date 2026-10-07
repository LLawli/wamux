//! Live validation for issue #26: a channel's history, and the tallies that
//! exist nowhere else.
//!
//! Read-only. Unlike the other `*_live` bins it sends nothing and needs no
//! `WAMUX_LIVE_DEST` guard: every call here is a read of a channel this account
//! already follows.
//!
//! Usage: newsletter_history_live [count]   (default 20)
//! Env: WAMUX_REF      account external_ref (required)
//!      WAMUX_SOCKET_PATH
//!      WAMUX_CHANNEL  one `@newsletter` jid; without it, every subscribed
//!                     channel is read in turn, which is what actually finds a
//!                     poll (they are rare, and the two in issue #26's store
//!                     were in two different channels).
//!
//! What it proves, and why each half is here:
//!   1. the page arrives and projects -- `server_id`, `type` and a payload that
//!      decoded, in the same `InboundMessage` shape the bus delivers;
//!   2. `before` really paginates -- the second page must be strictly OLDER than
//!      the first. A server that ignored the cursor would hand back page one
//!      again and every other assertion would still pass;
//!   3. `count = 0` is refused as InvalidArgument rather than answering an
//!      empty list, which would read as "this channel has no history";
//!   4. on a poll row, it MATCHES the server's vote tally against the poll's own
//!      options. The server names an option by a 32-byte hash and never by its
//!      text, so the claim "that hash is `sha256(option_name)`" is the one thing
//!      that makes a channel poll readable at all -- and it is checked here
//!      against the live server, not asserted from a fixture.

use std::fmt::Write as _;
use std::process::ExitCode;
use std::time::Duration;

use tonic::transport::Channel;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::newsletter_service_client::NewsletterServiceClient;
use wamux_tools::live_env::{account_ref_from, process_env, socket_path_from};
use wamux_tools::report::Report;
use wamux_tools::socket_client::{account_ref, connect_uds, wait_connected};
use whatsapp_rust::buffa::Message as _;
use whatsapp_rust::waproto::whatsapp as wa;

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let external_ref: String = account_ref_from(&process_env)?;
    let socket: String = socket_path_from(&process_env)?;
    let count: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);

    let channel = connect_uds(&socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut newsletters = NewsletterServiceClient::new(channel);
    let acct = account_ref(&external_ref);
    let mut report = Report::new();

    let own = wait_connected(&mut account, &acct, Duration::from_secs(30)).await?;
    println!("connected as {own}");
    let targets = channels_to_read(&mut newsletters, &mut report, &acct).await;
    println!("reading {} channel(s)\n", targets.len());

    let mut polls = Vec::new();
    for (jid, name) in &targets {
        polls.extend(read_one(&mut newsletters, &mut report, &acct, jid, name, count).await);
    }

    // The cursor and the guard are properties of the RPC, not of a channel, so
    // they run once against whichever channel actually returned history.
    if let Some((jid, name)) = targets.first() {
        check_pagination(&mut newsletters, &mut report, &acct, jid, name, count).await;
        check_zero_count(&mut newsletters, &mut report, &acct, jid).await;
    }
    report_polls(&polls);
    Ok(report.finish())
}

/// The channels to read: the one named in `WAMUX_CHANNEL`, or all of them.
async fn channels_to_read(
    newsletters: &mut NewsletterServiceClient<Channel>,
    report: &mut Report,
    acct: &pb::AccountRef,
) -> Vec<(String, String)> {
    let subscribed = match newsletters.list_subscribed_newsletters(acct.clone()).await {
        Ok(listed) => listed.into_inner().newsletters,
        Err(status) => {
            report.fail("Newsletter.ListSubscribedNewsletters", status.to_string());
            return Vec::new();
        }
    };
    report.verify(
        "Newsletter.ListSubscribedNewsletters",
        !subscribed.is_empty(),
        format!("subscribed to {} channel(s)", subscribed.len()),
    );
    let all: Vec<(String, String)> = subscribed
        .into_iter()
        .map(|found| {
            (
                found.jid.map(|jid| jid.value).unwrap_or_default(),
                found.name,
            )
        })
        .collect();
    let Some(wanted) = process_env("WAMUX_CHANNEL") else {
        return all;
    };
    match all.iter().find(|(jid, _)| jid == &wanted) {
        Some(found) => vec![found.clone()],
        // Not subscribed is not a reason to stop: the IQ may still answer, and
        // a refusal is itself the measurement.
        None => vec![(wanted, String::new())],
    }
}

/// One page of one channel. Returns the poll rows it saw.
async fn read_one(
    newsletters: &mut NewsletterServiceClient<Channel>,
    report: &mut Report,
    acct: &pb::AccountRef,
    jid: &str,
    name: &str,
    count: u32,
) -> Vec<String> {
    let check = format!("Newsletter.GetNewsletterMessages {name} <{jid}>");
    let page = match fetch(newsletters, acct, jid, count, 0).await {
        Ok(page) => page,
        Err(status) => {
            report.fail(&check, status.message());
            return Vec::new();
        }
    };
    report.verify(&check, !page.is_empty(), format!("{} row(s)", page.len()));
    for row in &page {
        println!("   {}", describe(row));
    }
    poll_rows(&page, report, jid, name)
}

/// The poll rows in one page, each already checked against its own options.
fn poll_rows(
    page: &[pb::NewsletterMessage],
    report: &mut Report,
    jid: &str,
    name: &str,
) -> Vec<String> {
    page.iter()
        .filter(|row| row.r#type() == pb::NewsletterMessageType::Poll)
        .map(|row| describe_poll(row, report, jid, name))
        .collect()
}

/// A poll row, with the tally resolved back to option names.
///
/// This is the measurement issue #26 turns on: the server sends `<vote
/// count="N">` with 32 opaque bytes and never the option's text. If those bytes
/// are `sha256(option_name)`, a consumer can render the result with nothing but
/// the poll's own payload -- no key, no secret, no extra round trip.
fn describe_poll(
    row: &pb::NewsletterMessage,
    report: &mut Report,
    jid: &str,
    name: &str,
) -> String {
    let options = poll_options(row);
    let mut out = format!(
        "{} <{jid}> [{}/{}] server_id={} {} option(s), {} vote node(s)",
        display_name(name, jid),
        type_label(row),
        poll_type_label(row),
        row.server_id,
        options.len(),
        row.votes.len(),
    );
    for vote in &row.votes {
        let named = options
            .iter()
            .find(|option| wacore::poll::compute_option_hash(option) == vote.option_hash[..]);
        let check = format!(
            "poll vote hash is sha256(option) {jid} server_id={}",
            row.server_id
        );
        match named {
            Some(option) => {
                let _ = write!(out, "\n      \u{2705} {:>7} \"{option}\"", vote.count);
                report.pass(&check, format!("{} vote(s) for {option:?}", vote.count));
            }
            None => {
                let why = format!(
                    "hash {} matches no option of this poll",
                    hex(&vote.option_hash)
                );
                let _ = write!(out, "\n      \u{274C} {:>7} {why}", vote.count);
                report.fail(&check, why);
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

/// The row's `type` as its enum name, plus the server's token when UNKNOWN.
fn type_label(row: &pb::NewsletterMessage) -> String {
    with_raw(row.r#type().as_str_name(), &row.type_raw)
}

/// The row's `poll_type` as its enum name, plus the server's token when UNKNOWN.
fn poll_type_label(row: &pb::NewsletterMessage) -> String {
    with_raw(row.poll_type().as_str_name(), &row.poll_type_raw)
}

fn with_raw(name: &str, raw: &str) -> String {
    if raw.is_empty() {
        return name.to_string();
    }
    format!("{name}({raw})")
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
        type_label(row),
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
    report: &mut Report,
    acct: &pb::AccountRef,
    jid: &str,
    name: &str,
    count: u32,
) {
    const NAME: &str = "Newsletter.GetNewsletterMessages before=cursor";
    let first = fetch(newsletters, acct, jid, count, 0)
        .await
        .unwrap_or_default();
    let Some(oldest) = first.iter().map(|row| row.server_id).min() else {
        return report.accepted(NAME, format!("{name} returned no rows, cursor untested"));
    };
    let second = fetch(newsletters, acct, jid, count, oldest)
        .await
        .unwrap_or_default();
    match second.iter().map(|row| row.server_id).max() {
        Some(newest) => report.verify(
            NAME,
            newest < oldest,
            format!("before={oldest}: next page tops out at {newest}"),
        ),
        None => report.accepted(
            NAME,
            format!("before={oldest} returned nothing (start of history)"),
        ),
    }
}

/// A zero page size asks the server for nothing; the core must refuse it.
async fn check_zero_count(
    newsletters: &mut NewsletterServiceClient<Channel>,
    report: &mut Report,
    acct: &pb::AccountRef,
    jid: &str,
) {
    const NAME: &str = "Newsletter.GetNewsletterMessages count=0";
    match fetch(newsletters, acct, jid, 0, 0).await {
        Err(status) if status.code() == tonic::Code::InvalidArgument => {
            report.pass(NAME, format!("refused: {}", status.message()))
        }
        Err(status) => report.fail(
            NAME,
            format!("gave {:?}: {}", status.code(), status.message()),
        ),
        Ok(page) => report.fail(
            NAME,
            format!("answered {} row(s) instead of failing", page.len()),
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
            jid: Some(pb::Jid {
                value: jid.to_string(),
            }),
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
