//! Live validation for issue #28: answering a button, a list or a template.
//!
//! The reply path cannot be proved between two accounts paired on one daemon —
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
//! must match `WAMUX_LIVE_DEST` — the same guard `stress_live`, `poll_live` and
//! `read_receipt_live` use. Without `WAMUX_REPLY` this bin only ever reads.
//!
//! Usage: interactive_reply_live [socket_path] [seconds]
//!   defaults: /tmp/wamux.sock 600
//! Env: WAMUX_REF        account external_ref (default "pair-socket")
//!      WAMUX_LIVE_DEST  the conversation to answer in; required to send
//!      WAMUX_REPLY      the choice to pick, as printed (e.g. "2"). Read-only
//!                       when unset.
//!
//! What to look for: the merchant's bot moves on. A `SendResult` means the
//! library accepted the message and an ack means the server did — neither means
//! the bot understood the id. The proof is the next message in the chat.

use std::time::Duration;

use hyper_util::rt::TokioIo;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use wamux::proto::v1 as pb;
use wamux::proto::v1::account_service_client::AccountServiceClient;
use wamux::proto::v1::event_service_client::EventServiceClient;
use wamux::proto::v1::messaging_service_client::MessagingServiceClient;
use whatsapp_rust::buffa::Message as _;
use whatsapp_rust::waproto::whatsapp as wa;

const DEFAULT_REF: &str = "pair-socket";

/// One answerable choice out of an offer, already in the shape the RPC wants.
struct Choice {
    label: String,
    reply: pb::send_interactive_reply_request::Reply,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket = arg(1, "/tmp/wamux.sock");
    let secs: u64 = arg(2, "600").parse().unwrap_or(600);
    let pick = std::env::var("WAMUX_REPLY").ok();
    let dest = std::env::var("WAMUX_LIVE_DEST").ok();
    if pick.is_some() && dest.is_none() {
        anyhow::bail!("WAMUX_REPLY needs WAMUX_LIVE_DEST: a reply reaches a real bot");
    }

    let channel = connect_uds(socket).await?;
    let mut account = AccountServiceClient::new(channel.clone());
    let mut messaging = MessagingServiceClient::new(channel.clone());
    let mut events = EventServiceClient::new(channel);
    let acct = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::ExternalRef(
            std::env::var("WAMUX_REF").unwrap_or_else(|_| DEFAULT_REF.to_string()),
        )),
    };

    let own = wait_connected(&mut account, &acct).await?;
    println!("connected as {own}");
    match (&pick, &dest) {
        (Some(choice), Some(to)) => {
            println!("will answer the first offer from {to} with #{choice}")
        }
        _ => println!("read-only: set WAMUX_REPLY + WAMUX_LIVE_DEST to actually answer"),
    }

    let mut stream = events
        .subscribe_events(pb::SubscribeRequest {
            selector: Some(pb::subscribe_request::Selector::Account(acct.clone())),
            replay_from_ring: 0,
        })
        .await?
        .into_inner();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    while tokio::time::Instant::now() < deadline {
        let Ok(Ok(Some(envelope))) =
            tokio::time::timeout(Duration::from_secs(5), stream.message()).await
        else {
            continue;
        };
        let Some(pb::event_envelope::Event::Message(inbound)) = envelope.event else {
            continue;
        };
        let choices = offer_choices(&inbound);
        if choices.is_empty() {
            continue;
        }
        report_offer(&inbound, &choices);
        if let (Some(choice), Some(to)) = (&pick, &dest)
            && same_user(&inbound.chat, to)
        {
            answer(&mut messaging, &acct, &inbound, &choices, choice, to).await;
            return Ok(());
        }
    }
    println!("\u{23F0} no offer arrived in {secs}s");
    Ok(())
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
/// The hydrated template rides in EITHER slot and usually both — `hydrated_
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

/// Send the chosen reply, quoting the offer and carrying its payload.
async fn answer(
    messaging: &mut MessagingServiceClient<Channel>,
    acct: &pb::AccountRef,
    inbound: &pb::InboundMessage,
    choices: &[Choice],
    pick: &str,
    to: &str,
) {
    let Ok(index) = pick.parse::<usize>() else {
        println!("\u{274C} WAMUX_REPLY={pick:?} is not a number");
        return;
    };
    let Some(choice) = choices.get(index) else {
        println!(
            "\u{274C} #{index} is out of range ({} choices)",
            choices.len()
        );
        return;
    };
    let Some(key) = inbound.key.clone() else {
        println!("\u{274C} the offer carries no key to quote");
        return;
    };
    let request = pb::SendInteractiveReplyRequest {
        account: Some(acct.clone()),
        to: Some(pb::Jid {
            value: to.to_string(),
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
    match messaging.send_interactive_reply(request).await {
        Ok(response) => {
            let sent = response.into_inner();
            let id = sent.key.map(|key| key.id).unwrap_or_default();
            println!("\u{2705} answered #{index} ({}) as {id}", choice.label);
            println!("   the proof is the bot's NEXT message, not this id.");
        }
        Err(status) => println!("\u{274C} {}: {}", status.code(), status.message()),
    }
}

fn arg(index: usize, fallback: &str) -> String {
    std::env::args()
        .nth(index)
        .unwrap_or_else(|| fallback.to_string())
}

/// The user part of a jid, without the server or any device suffix: the chat
/// may arrive `@lid` while the guard names a phone jid.
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
