//! Live validation for issue #28: answering a button, a list or a template.
//!
//! The reply path cannot be proved between two accounts paired on one daemon --
//! neither can offer the other a button. The first real proof is a tap in a
//! conversation with a real business, so this bin is built to make that one
//! command instead of a session of hand-rolled gRPC.
//!
//! It watches the event stream, decodes every offer that arrives, and prints
//! each choice with the id a reply has to name. With `WAMUX_REPLY` set it then
//! answers the first offer it sees with that choice.
//!
//! SAFETY: a reply LEAVES THE MACHINE and reaches a real merchant's bot. So the
//! send half is off unless `WAMUX_REPLY` names a choice, and the destination
//! must match `WAMUX_LIVE_DEST` -- the same guard `stress_live`, `poll_live` and
//! `read_receipt_live` use. Without `WAMUX_REPLY` this bin only ever reads.
//!
//! Usage: interactive_reply_live [seconds]   (default 600)
//! Env: WAMUX_REF        account external_ref (required)
//!      WAMUX_LIVE_DEST  the conversation to answer in; required only when
//!                       WAMUX_REPLY is set
//!      WAMUX_REPLY      the choice to pick, as printed (e.g. "2"). Read-only
//!                       when unset.
//!      WAMUX_DELIVERY_SECS  how long a reply waits for its `delivered` receipt
//!
//! What to look for: the merchant's bot moves on. A `SendResult` means the
//! library accepted the message and an ack means the server did -- neither means
//! the bot understood the id. The proof is the next message in the chat.

use std::process::ExitCode;
use std::time::Duration;

use tonic::transport::Channel;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;
use wamux_tools::delivery::{EventTap, send_reached_phone};
use wamux_tools::live_env::{
    account_ref_from, delivery_window_from, live_dest_from, process_env, refuse_own_number,
    same_user, socket_path_from,
};
use wamux_tools::report::Report;
use wamux_tools::socket_client::{account_ref, connect_uds, wait_connected};
use whatsapp_rust::buffa::Message as _;
use whatsapp_rust::waproto::whatsapp as wa;

/// One answerable choice out of an offer, already in the shape the RPC wants.
struct Choice {
    label: String,
    reply: pb::send_interactive_reply_request::Reply,
}

/// How the watch loop re-reads the tap for new events.
const TAP_POLL: Duration = Duration::from_millis(500);

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let external_ref: String = account_ref_from(&process_env)?;
    let socket: String = socket_path_from(&process_env)?;
    let window: Duration = delivery_window_from(&process_env)?;
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(600);
    let pick: Option<String> = process_env("WAMUX_REPLY");
    // A reply reaches a real bot, so the destination is required exactly then.
    let dest: Option<String> = match pick {
        Some(_) => Some(live_dest_from(&process_env)?),
        None => None,
    };

    let channel = connect_uds(&socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut messaging = MessagingServiceClient::new(channel.clone());
    let mut events = EventServiceClient::new(channel);
    let acct = account_ref(&external_ref);

    let own = wait_connected(&mut account, &acct, Duration::from_secs(30)).await?;
    println!("connected as {own}");
    if let Some(to) = &dest {
        refuse_own_number(&own, to)?;
    }
    match (&pick, &dest) {
        (Some(choice), Some(to)) => {
            println!("will answer the first offer from {to} with #{choice}")
        }
        _ => println!("read-only: set WAMUX_REPLY + WAMUX_LIVE_DEST to actually answer"),
    }

    let stream = events
        .subscribe_events(pb::SubscribeRequest {
            selector: Some(pb::subscribe_request::Selector::Account(acct.clone())),
            replay_from_ring: 0,
        })
        .await?
        .into_inner();
    let tap = EventTap::spawn(stream);
    let mut report = Report::new();
    let reply = pick.as_deref().zip(dest.as_deref());
    let found = watch_offers(&tap, &mut report, secs, reply).await;
    let Some((inbound, choices, (pick, to))) = found else {
        return Ok(finish_without_reply(report, pick.is_some(), secs));
    };
    let ctx = Answer {
        acct: &acct,
        tap: &tap,
        window,
        to,
    };
    answer(&mut messaging, &mut report, &ctx, &inbound, &choices, pick).await;
    Ok(report.finish())
}

/// What a reply needs besides the offer: who answers, where, and how long to
/// wait for delivery.
struct Answer<'a> {
    acct: &'a pb::AccountRef,
    tap: &'a EventTap,
    window: Duration,
    to: &'a str,
}

/// Read-only runs prove only that an offer was decoded; a run that waited in
/// vain, or never reached the answer, is a failure.
fn finish_without_reply(mut report: Report, replying: bool, secs: u64) -> ExitCode {
    if report.checks().is_empty() {
        report.fail(
            "Event.InboundMessage offer",
            format!("no offer arrived in {secs}s"),
        );
    } else if replying {
        report.fail(
            "Messaging.SendInteractiveReply",
            format!("no offer from WAMUX_LIVE_DEST arrived in {secs}s"),
        );
    }
    report.finish()
}

type Found<'a> = (pb::InboundMessage, Vec<Choice>, (&'a str, &'a str));

/// Print every offer that arrives. Returns the first one from the destination
/// when a reply was asked for; otherwise it watches until the window closes
/// and returns `None`.
async fn watch_offers<'a>(
    tap: &EventTap,
    report: &mut Report,
    secs: u64,
    reply: Option<(&'a str, &'a str)>,
) -> Option<Found<'a>> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    let mut next = 0usize;
    while tokio::time::Instant::now() < deadline {
        let seen = tap.seen();
        for envelope in &seen[next..] {
            let Some(pb::event_envelope::Event::Message(inbound)) = &envelope.event else {
                continue;
            };
            let choices = offer_choices(inbound);
            if choices.is_empty() {
                continue;
            }
            report_offer(inbound, &choices);
            report.pass(
                "Event.InboundMessage offer",
                format!("{} choices in {}", choices.len(), inbound.chat),
            );
            if let Some(chosen) = reply
                && same_user(&inbound.chat, chosen.1)
            {
                return Some((inbound.clone(), choices, chosen));
            }
        }
        next = seen.len();
        tokio::time::sleep(TAP_POLL).await;
    }
    None
}

/// Every answerable choice in this message, or nothing if it is not an offer.
///
/// A `cta_url` button is deliberately absent: tapping it opens a link and
/// nothing leaves the machine, so it is not this RPC's business (issue #28).
fn offer_choices(inbound: &pb::InboundMessage) -> Vec<Choice> {
    let Ok(message) = wa::Message::decode(&mut inbound.raw_message.as_slice()) else {
        return Vec::new();
    };
    if let Some(buttons) = message.buttons_message.as_option() {
        return button_choices(buttons);
    }
    if let Some(list) = message.list_message.as_option() {
        return list_choices(list);
    }
    if let Some(template) = message.template_message.as_option() {
        return template_choices(template);
    }
    Vec::new()
}

fn button_choices(offer: &wa::message::ButtonsMessage) -> Vec<Choice> {
    offer
        .buttons
        .iter()
        .map(|button| {
            let id = button.button_id.clone().unwrap_or_default();
            let text = button
                .button_text
                .as_option()
                .and_then(|t| t.display_text.clone())
                .unwrap_or_default();
            Choice {
                label: format!("button  id={id:?} text={text:?}"),
                reply: pb::send_interactive_reply_request::Reply::Button(pb::ButtonReply {
                    selected_id: id,
                    display_text: text,
                }),
            }
        })
        .collect()
}

fn list_choices(offer: &wa::message::ListMessage) -> Vec<Choice> {
    offer
        .sections
        .iter()
        .flat_map(|section| section.rows.iter())
        .map(|row| {
            let id = row.row_id.clone().unwrap_or_default();
            let title = row.title.clone().unwrap_or_default();
            Choice {
                label: format!("list    row_id={id:?} title={title:?}"),
                reply: pb::send_interactive_reply_request::Reply::List(pb::ListReply {
                    selected_row_id: id,
                    title,
                    description: row.description.clone().unwrap_or_default(),
                }),
            }
        })
        .collect()
}

/// Only the quick-reply buttons: a URL or call button is not answered by a
/// message, so offering it here would invite a reply nobody expects. Measured
/// over 143 live template offers: 96 carry a quick reply, 40 carry only a URL.
///
/// The hydrated template rides in EITHER slot and usually both -- `hydrated_
/// template` (field 4) in 56 of those 143, the `hydratedFourRowTemplate` arm of
/// the `format` oneof (field 2) in 105. Reading one slot would have silently
/// skipped a third of the offers.
fn template_choices(offer: &wa::message::TemplateMessage) -> Vec<Choice> {
    use wa::message::template_message::Format;
    let hydrated = offer
        .hydrated_template
        .as_option()
        .or_else(|| match offer.format.as_ref() {
            Some(Format::HydratedFourRowTemplate(found)) => Some(found.as_ref()),
            _ => None,
        });
    let Some(hydrated) = hydrated else {
        return Vec::new();
    };
    hydrated
        .hydrated_buttons
        .iter()
        .filter_map(|button| {
            use wa::hydrated_template_button::HydratedButton;
            let index = button.index.unwrap_or(0);
            let Some(HydratedButton::QuickReplyButton(quick)) = button.hydrated_button.as_ref()
            else {
                return None;
            };
            let id = quick.id.clone().unwrap_or_default();
            let text = quick.display_text.clone().unwrap_or_default();
            Some(Choice {
                label: format!("template id={id:?} text={text:?} index={index}"),
                reply: pb::send_interactive_reply_request::Reply::Template(pb::TemplateReply {
                    selected_id: id,
                    display_text: text,
                    selected_index: index,
                }),
            })
        })
        .collect()
}

fn report_offer(inbound: &pb::InboundMessage, choices: &[Choice]) {
    let id = inbound
        .key
        .as_ref()
        .map(|key| key.id.as_str())
        .unwrap_or("");
    println!("\n\u{1F4E9} offer in {} (id {id})", inbound.chat);
    for (index, choice) in choices.iter().enumerate() {
        println!("   #{index}  {}", choice.label);
    }
}

/// Send the chosen reply, quoting the offer and carrying its payload. The
/// reply passes only when it reached the bot's phone and a `delivered`
/// receipt came back; the bot's NEXT message remains the human proof.
async fn answer(
    messaging: &mut MessagingServiceClient<Channel>,
    report: &mut Report,
    ctx: &Answer<'_>,
    inbound: &pb::InboundMessage,
    choices: &[Choice],
    pick: &str,
) {
    const NAME: &str = "Messaging.SendInteractiveReply";
    let Ok(index) = pick.parse::<usize>() else {
        return report.fail(NAME, format!("WAMUX_REPLY={pick:?} is not a number"));
    };
    let Some(choice) = choices.get(index) else {
        let why = format!("#{index} is out of range ({} choices)", choices.len());
        return report.fail(NAME, why);
    };
    let Some(key) = inbound.key.clone() else {
        return report.fail(NAME, "the offer carries no key to quote");
    };
    let request = pb::SendInteractiveReplyRequest {
        account: Some(ctx.acct.clone()),
        to: Some(pb::Jid {
            value: ctx.to.to_string(),
        }),
        quote: Some(pb::QuoteContext {
            quoted: Some(key),
            participant: inbound.sender.clone(),
        }),
        // The official client embeds the whole offer, not just its id, and the
        // edge already holds those bytes. Relayed here so the live shape
        // matches the captured one.
        quoted_message: inbound.raw_message.clone(),
        reply: Some(choice.reply.clone()),
    };
    let sent = match messaging.send_interactive_reply(request).await {
        Ok(response) => response.into_inner(),
        Err(status) => {
            return report.fail(NAME, format!("{}: {}", status.code(), status.message()));
        }
    };
    let id = match send_reached_phone(&sent) {
        Ok(id) => id,
        Err(why) => return report.fail(NAME, why),
    };
    let delivered = ctx.tap.delivered(&id, ctx.window).await;
    report.verify(
        NAME,
        delivered,
        format!(
            "answered #{index} ({}) as {id}; delivered={delivered}",
            choice.label
        ),
    );
}
