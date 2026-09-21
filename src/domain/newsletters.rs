//! Channel (`@newsletter`) metadata, relayed over `client.newsletter()`.
//!
//! Issue #6: a channel's name existed on WhatsApp's side and never crossed the
//! contract. An inbound newsletter message carries an empty `push_name` and a
//! `sender` equal to the newsletter jid, history sync brings no name, and
//! `GroupService` does not cover them — so a consumer could only ever address
//! these rows by jid. Nothing new is implemented against WhatsApp here; the
//! library already asks, the core just never relayed the answer.

use serde_json::json;
use wacore::iq::newsletter::NEWSLETTER_XMLNS;
use wacore::request::InfoQuery;
use whatsapp_rust::buffa::Message as _;
use whatsapp_rust::features::MexRequest;
use whatsapp_rust::wacore_binary::NodeContent;
use whatsapp_rust::wacore_binary::jid::SERVER_JID;
use whatsapp_rust::wacore_binary::{NodeContentRef, NodeRef};
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::{Client, Jid, NodeBuilder};

use crate::domain::event_mapping::project_content;
use crate::domain::jid_parse::parse_jid;
use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

// WORKAROUND (upstream oxidezap/whatsapp-rust#1372) — REMOVE WHEN FIXED.
//
// `client.newsletter().list_subscribed()` and `.get_metadata()` cannot be used:
// their generated `Variables` mark every field `skip_serializing_if =
// "Option::is_none"`, and the library passes them unset, so the request carries
// `{}` (list) or a partial object (get). These persisted GraphQL operations
// require EVERY declared variable to be present, and the server answers
// `400 Bad Request` when one is missing. Measured against the live accounts:
// `{}` and any partial set fail; the complete set succeeds regardless of the
// booleans' values.
//
// So the calls below issue the same persisted operations through the public
// `MexRequest` API with every variable populated, and parse the answer here.
// The doc ids are the ones the library generated (identical in 0.7.0 and in
// upstream main), NOT ones this project scraped.
//
// The moment #1372 lands, delete `list_subscribed`'s and `get_metadata`'s bodies
// here and call the library again: nothing else in this module changes, because
// the projection below is what the RPC returns either way.
const LIST_QUERY: (&str, &str) = (
    "WAWebMexFetchAllNewslettersMetadataJobQuery",
    "25399611239711790",
);
const GET_QUERY: (&str, &str) = ("WAWebMexFetchNewsletterJobQuery", "27456920720571478");

pub async fn list_subscribed(client: &Client) -> Result<pb::NewsletterList, WamuxError> {
    let response = client
        .mex()
        .query(MexRequest::new(
            LIST_QUERY.0,
            LIST_QUERY.1,
            // Every declared variable, present. See the note above: absence is
            // what the server rejects, not the value.
            json!({ "fetch_status_metadata": true, "fetch_wamo_sub": true }),
        ))
        .await
        .map_err(client_err)?;

    let data = response
        .data
        .ok_or_else(|| WamuxError::Client("newsletter list: no data".to_string()))?;
    let found = data["xwa2_newsletter_subscribed"]
        .as_array()
        .ok_or_else(|| {
            WamuxError::Client("newsletter list: missing xwa2_newsletter_subscribed".to_string())
        })?;
    Ok(pb::NewsletterList {
        newsletters: found.iter().map(newsletter_to_proto).collect(),
    })
}

pub async fn get_metadata(client: &Client, jid: &str) -> Result<pb::Newsletter, WamuxError> {
    // Parsed for validation only: the query addresses the channel by string.
    let jid = parse_jid(jid)?;
    let response = client
        .mex()
        .query(MexRequest::new(
            GET_QUERY.0,
            GET_QUERY.1,
            json!({
                "input": { "key": jid.to_string(), "type": "JID", "view_role": "GUEST" },
                "fetch_creation_time": true,
                "fetch_full_image": true,
                "fetch_pinned_messages": false,
                "fetch_status_metadata": false,
                "fetch_viewer_metadata": true,
                "fetch_wamo_sub": false,
            }),
        ))
        .await
        .map_err(client_err)?;

    let data = response
        .data
        .ok_or_else(|| WamuxError::Client("newsletter: no data".to_string()))?;
    let found = &data["xwa2_newsletter"];
    if found.is_null() {
        return Err(WamuxError::AccountNotFound(format!(
            "no newsletter metadata for {jid}"
        )));
    }
    Ok(newsletter_to_proto(found))
}

// WORKAROUND (upstream whatsapp-rust 0.7.0, NOT YET REPORTED) — REMOVE WHEN FIXED.
//
// `client.newsletter().get_messages()` cannot be used, for two independent
// reasons, both measured live on 2026-09-21 against the production accounts.
//
// 1. It addresses the IQ wrong, and the server does not answer AT ALL. The
//    library sends `to = <channel>` with `<messages count="N"/>`. WA Web (and
//    whatsmeow) send it to the server and name the channel on the inner node:
//
//      <iq to="s.whatsapp.net" xmlns="newsletter" type="get">
//        <messages type="jid" jid="<channel>" count="N" [before="M"]/>
//      </iq>
//
//    The library's spelling draws no stanza back — not an error stanza,
//    silence — and 22-33s later the keepalive watchdog declares the socket dead
//    and reconnects the whole account. One history read takes the connection
//    down. The corrected spelling answers in well under a second.
//
// 2. Its parser reads only `<plaintext>` and `<reactions>` off each `<message>`
//    and drops every other child, so even a working call would lose exactly
//    what this RPC exists for. The server really sends:
//
//      <message server_id="777" id="AC3C..." type="poll" t="1790001172">
//        <forwards_count count="750"/>
//        <meta polltype="creation"/>
//        <votes><vote count="25328">..32 bytes..</vote> ...</votes>
//        <reactions/>
//        <plaintext>..</plaintext>
//      </message>
//
//    `<votes>` is the channel poll tally — the thing issue #26 went looking
//    for. The library's `NewsletterMessageType` does not even have `poll`.
//
// So the IQ is built and parsed here, the same shape the workaround above uses
// for the MEX queries. When this is fixed upstream, it goes back to one library
// call and the projection below stays as it is.

/// A page of a channel's history, newest first (issue #26).
///
/// Plain IQ, nothing decrypts: a channel is not E2E, so the server hands back
/// the payloads itself. It is also the only place the server's own tallies
/// exist — reactions, poll votes and forwards are counted server-side and ride
/// on no payload — which is the whole reason this RPC is here.
pub async fn get_messages(
    client: &Client,
    req: &pb::GetNewsletterMessagesRequest,
) -> Result<pb::NewsletterMessageList, WamuxError> {
    let jid = parse_jid(&req.jid)?;
    require_count(req.count)?;
    // 0 is proto3's "absent": start at the newest rather than before row zero.
    let before = (req.before != 0).then_some(req.before);
    let response = client
        .send_iq(history_query(&jid, req.count, before)?)
        .await
        .map_err(client_err)?;
    Ok(pb::NewsletterMessageList {
        messages: parse_history(response.get(), &req.jid),
    })
}

/// The IQ WA Web sends: addressed to the server, with the channel named on the
/// inner node. See the workaround note above for what the other spelling does.
fn history_query(
    jid: &Jid,
    count: u32,
    before: Option<u64>,
) -> Result<InfoQuery<'static>, WamuxError> {
    let mut messages = NodeBuilder::new("messages")
        .attr("type", "jid")
        .attr("jid", jid.to_string())
        .attr("count", count);
    if let Some(cursor) = before {
        messages = messages.attr("before", cursor);
    }
    let server: Jid = SERVER_JID
        .parse()
        .map_err(|err| WamuxError::Client(format!("server jid {SERVER_JID}: {err}")))?;
    Ok(InfoQuery::get(
        NEWSLETTER_XMLNS,
        server,
        Some(NodeContent::Nodes(vec![messages.build()])),
    ))
}

/// The page size is the caller's to choose, and 0 asks the server for nothing.
/// Refused here so it answers InvalidArgument instead of an empty list, which
/// reads as "this channel has no history".
fn require_count(count: u32) -> Result<(), WamuxError> {
    if count == 0 {
        return Err(WamuxError::InvalidArgument(
            "count must be at least 1, got 0".to_string(),
        ));
    }
    Ok(())
}

/// Every `<message>` under `<messages>`. A response shaped otherwise yields an
/// empty page rather than an error: the IQ succeeded, and a channel with no
/// history looks exactly like this.
fn parse_history(response: &NodeRef<'_>, chat: &str) -> Vec<pb::NewsletterMessage> {
    let Some(messages) = response.get_optional_child("messages") else {
        return Vec::new();
    };
    let Some(children) = messages.children() else {
        return Vec::new();
    };
    children
        .iter()
        .filter(|node| node.tag.as_ref() == "message")
        .filter_map(|node| message_node_to_proto(node, chat))
        .collect()
}

/// Project one `<message>` row onto the wire shape.
///
/// A row without a `server_id` is skipped, not defaulted: `server_id` is the
/// pagination cursor and the key the server's add-ons hang off, so a row that
/// has none cannot be addressed at all and a 0 would collide with every other
/// such row.
fn message_node_to_proto(node: &NodeRef<'_>, chat: &str) -> Option<pb::NewsletterMessage> {
    let server_id = attr_u64(node, "server_id")?;
    Some(pb::NewsletterMessage {
        message: Some(history_row_to_inbound(node, chat)),
        server_id,
        r#type: attr_str(node, "type").unwrap_or_default(),
        reactions: reaction_counts(node),
        votes: vote_counts(node),
        forwards_count: child_count(node, "forwards_count"),
        poll_type: node
            .get_optional_child("meta")
            .and_then(|meta| attr_str(meta, "polltype"))
            .unwrap_or_default(),
    })
}

/// The row's message, in the shape the event bus delivers.
///
/// Same projection a live event goes through (`project_content`), because a
/// channel payload IS the same `wa::Message` — it arrives as `<plaintext>`
/// bytes rather than encrypted. One concept, one shape.
///
/// `is_sender` is the only sender information the node carries, so it becomes
/// `key.from_me` and nothing fills `sender`, `push_name` or the alt-namespace
/// jids: a row the server described sparsely does not gain values it never had.
/// `chat` is the jid the caller addressed, not something read off the node.
fn history_row_to_inbound(node: &NodeRef<'_>, chat: &str) -> pb::InboundMessage {
    let payload = plaintext_payload(node);
    let mut out = pb::InboundMessage {
        key: Some(pb::MessageKey {
            remote_jid: chat.to_string(),
            id: attr_str(node, "id").unwrap_or_default(),
            from_me: attr_str(node, "is_sender").as_deref() == Some("true"),
            participant: String::new(),
        }),
        chat: chat.to_string(),
        timestamp: millis_from_seconds(attr_u64(node, "t").unwrap_or(0)),
        raw_message: payload
            .as_ref()
            .map(|message| message.encode_to_vec())
            .unwrap_or_default(),
        ..Default::default()
    };
    if let Some(message) = &payload {
        project_content(&mut out, message, chat);
    }
    out
}

/// The `<plaintext>` bytes, decoded. A channel is not E2E, so this is the whole
/// `wa::Message` with nothing to unwrap first.
fn plaintext_payload(node: &NodeRef<'_>) -> Option<wa::Message> {
    let plaintext = node.get_optional_child("plaintext")?;
    match plaintext.content.as_ref()? {
        NodeContentRef::Bytes(bytes) => {
            whatsapp_rust::waproto::codec::message_decode(bytes.as_ref()).ok()
        }
        _ => None,
    }
}

/// `<reactions><reaction code=".." count="N"/></reactions>`. A reaction with no
/// code is skipped: the count belongs to an emoji nobody can name.
fn reaction_counts(node: &NodeRef<'_>) -> Vec<pb::NewsletterReactionCount> {
    children_named(node, "reactions", "reaction")
        .filter_map(|reaction| {
            let code = attr_str(&reaction, "code").filter(|code| !code.is_empty())?;
            Some(pb::NewsletterReactionCount {
                code,
                count: attr_u64(&reaction, "count").unwrap_or(0),
            })
        })
        .collect()
}

/// `<votes><vote count="N">..32 bytes..</vote></votes>`. The bytes ARE the
/// option, hashed; a vote whose content is not bytes names no option and is
/// dropped rather than relayed as a count attached to nothing.
fn vote_counts(node: &NodeRef<'_>) -> Vec<pb::NewsletterPollVote> {
    children_named(node, "votes", "vote")
        .filter_map(|vote| {
            let NodeContentRef::Bytes(hash) = vote.content.as_ref()? else {
                return None;
            };
            Some(pb::NewsletterPollVote {
                option_hash: hash.to_vec(),
                count: attr_u64(&vote, "count").unwrap_or(0),
            })
        })
        .collect()
}

/// The `count` attribute of a single optional child, e.g. `<forwards_count/>`.
fn child_count(node: &NodeRef<'_>, tag: &str) -> u64 {
    node.get_optional_child(tag)
        .and_then(|child| attr_u64(child, "count"))
        .unwrap_or(0)
}

/// The `<inner>` children of an optional `<outer>` wrapper.
fn children_named<'a>(
    node: &'a NodeRef<'a>,
    outer: &'a str,
    inner: &'a str,
) -> impl Iterator<Item = NodeRef<'a>> + 'a {
    node.get_optional_child(outer)
        .and_then(|wrapper| wrapper.children().map(|found| found.to_vec()))
        .unwrap_or_default()
        .into_iter()
        .filter(move |child| child.tag.as_ref() == inner)
}

fn attr_str(node: &NodeRef<'_>, key: &str) -> Option<String> {
    node.get_attr(key).map(|value| value.as_str().into_owned())
}

fn attr_u64(node: &NodeRef<'_>, key: &str) -> Option<u64> {
    node.get_attr(key)
        .and_then(|value| value.as_str().parse::<u64>().ok())
}

/// The stanza counts in seconds; every timestamp in this contract is
/// milliseconds. Saturating, so a nonsense value cannot come back as a date in
/// the past.
fn millis_from_seconds(seconds: u64) -> i64 {
    i64::try_from(seconds)
        .unwrap_or(i64::MAX)
        .saturating_mul(1000)
}

/// Project one newsletter node onto the wire shape.
///
/// Mirrors the library's own `parse_newsletter_metadata` field for field, so the
/// answer does not change when the workaround above is removed. Absent values
/// relay as the proto3 default, never a placeholder: a channel the server
/// described sparsely does not gain values it never had.
fn newsletter_to_proto(value: &serde_json::Value) -> pb::Newsletter {
    let thread = &value["thread_metadata"];
    pb::Newsletter {
        jid: value["id"].as_str().unwrap_or_default().to_string(),
        name: thread["name"]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        description: thread["description"]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        // The server sends counts as strings.
        subscriber_count: thread["subscribers_count"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        // A direct_path, not a URL: the same shape MediaDescriptor carries, so
        // the edge fetches it the way it fetches any other media path.
        picture_url: thread["picture"]["direct_path"]
            .as_str()
            .or_else(|| thread["preview"]["direct_path"].as_str())
            .unwrap_or_default()
            .to_string(),
        verification: lowercase_token(&thread["verification"]),
        state: lowercase_token(&value["state"]["type"]),
        role: lowercase_token(&value["viewer_metadata"]["role"]),
        creation_time: thread["creation_time"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
    }
}

/// The server sends these as SCREAMING_CASE (`VERIFIED`, `ACTIVE`,
/// `SUBSCRIBER`). Relay them lowercased, the same convention
/// `ReceiptEvent.type` and `CallEvent.action` follow, so the edge codes against
/// the proto rather than against whatever casing the server chose. An unknown
/// value passes through lowercased rather than being folded into a known token.
fn lowercase_token(value: &serde_json::Value) -> String {
    value.as_str().unwrap_or_default().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One node shaped exactly like the live server's answer, trimmed to the
    /// fields this projection reads. Captured from the production accounts.
    fn node() -> serde_json::Value {
        json!({
            "id": "120363144038483540@newsletter",
            "state": { "type": "ACTIVE" },
            "thread_metadata": {
                "creation_time": "1688746895",
                "description": { "text": "WhatsApp's official channel." },
                "invite": "0029Va4K0PZ5a245NkngBA2M",
                "name": { "text": "WhatsApp" },
                "preview": { "direct_path": "/v/t61.24694-24/416962407.jpg" },
                "subscribers_count": "4242",
                "verification": "VERIFIED"
            },
            "viewer_metadata": { "role": "SUBSCRIBER" }
        })
    }

    // The whole point of issue #6: the name has to survive the projection.
    #[test]
    fn a_channel_node_relays_its_name_and_jid() {
        let out = newsletter_to_proto(&node());
        assert_eq!(out.jid, "120363144038483540@newsletter");
        assert_eq!(out.name, "WhatsApp");
        assert_eq!(out.description, "WhatsApp's official channel.");
        assert_eq!(out.creation_time, 1_688_746_895);
    }

    // The server sends counts as strings, not numbers.
    #[test]
    fn subscriber_count_parses_from_its_string_form() {
        assert_eq!(newsletter_to_proto(&node()).subscriber_count, 4242);
    }

    // SCREAMING_CASE on the wire, lowercase tokens in the contract.
    #[test]
    fn enums_relay_as_lowercase_wire_tokens() {
        let out = newsletter_to_proto(&node());
        assert_eq!(out.verification, "verified");
        assert_eq!(out.state, "active");
        assert_eq!(out.role, "subscriber");
    }

    // `picture` is absent on most channels; `preview` is what they do carry.
    #[test]
    fn picture_falls_back_to_the_preview_path() {
        assert_eq!(
            newsletter_to_proto(&node()).picture_url,
            "/v/t61.24694-24/416962407.jpg"
        );
        let mut with_picture = node();
        with_picture["thread_metadata"]["picture"] = json!({ "direct_path": "/v/full.jpg" });
        assert_eq!(
            newsletter_to_proto(&with_picture).picture_url,
            "/v/full.jpg"
        );
    }

    // A channel the server described sparsely must not gain invented values.
    #[test]
    fn a_bare_node_stays_at_proto3_defaults() {
        let out = newsletter_to_proto(&json!({ "id": "1@newsletter" }));
        assert_eq!(out.jid, "1@newsletter");
        assert!(out.name.is_empty());
        assert!(out.description.is_empty());
        assert!(out.verification.is_empty());
        assert_eq!(out.subscriber_count, 0);
        assert_eq!(out.creation_time, 0);
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;
    use whatsapp_rust::wacore_binary::Node;

    const CHANNEL: &str = "120363144038483540@newsletter";
    const POLL_OPTION: &str = "\u{2600}\u{FE0F} GOOD MORNING EVERYONE LET'S GO!!!";

    /// The `<iq>` the server really answers, transcribed from the live probe of
    /// 2026-09-21 (issue #26): one media row and one poll row, with the tally
    /// nodes the library's own parser throws away.
    fn response() -> Node {
        NodeBuilder::new("iq")
            .attr("from", "s.whatsapp.net")
            .attr("type", "result")
            .children([NodeBuilder::new("messages")
                .attr("jid", CHANNEL)
                .children([media_row(), poll_row()])
                .build()])
            .build()
    }

    fn media_row() -> Node {
        NodeBuilder::new("message")
            .attr("server_id", "773")
            .attr("id", "3AEC8E07F1928E3484F7")
            .attr("type", "media")
            .attr("t", "1788795311")
            .children([
                NodeBuilder::new("forwards_count")
                    .attr("count", "8329")
                    .build(),
                NodeBuilder::new("reactions")
                    .children([
                        NodeBuilder::new("reaction")
                            .attr("code", "\u{1F621}")
                            .attr("count", "437")
                            .build(),
                        // No code: a count attached to an emoji nobody can name.
                        NodeBuilder::new("reaction").attr("count", "9").build(),
                    ])
                    .build(),
                NodeBuilder::new("plaintext")
                    .attr("mediatype", "gif")
                    .bytes(text_payload("bom dia"))
                    .build(),
            ])
            .build()
    }

    fn poll_row() -> Node {
        NodeBuilder::new("message")
            .attr("server_id", "777")
            .attr("id", "AC3CCD33792F68AC6D347EB1EBB74DE7")
            .attr("type", "poll")
            .attr("t", "1790001172")
            .children([
                NodeBuilder::new("forwards_count")
                    .attr("count", "750")
                    .build(),
                NodeBuilder::new("meta")
                    .attr("polltype", "creation")
                    .build(),
                NodeBuilder::new("votes")
                    .children([
                        NodeBuilder::new("vote")
                            .attr("count", "25328")
                            .bytes(option_hash(POLL_OPTION))
                            .build(),
                        NodeBuilder::new("vote").attr("count", "22950").build(),
                    ])
                    .build(),
                NodeBuilder::new("reactions").build(),
                NodeBuilder::new("plaintext")
                    .bytes(text_payload("qual a sua?"))
                    .build(),
            ])
            .build()
    }

    fn text_payload(text: &str) -> Vec<u8> {
        wa::Message {
            conversation: Some(text.to_string()),
            ..Default::default()
        }
        .encode_to_vec()
    }

    /// The library's own hash, not a reimplementation: `compute_option_hash` is
    /// what a poll vote references an option by, and the assertion below is
    /// that the server's `<vote>` bytes are exactly that.
    fn option_hash(option: &str) -> Vec<u8> {
        wacore::poll::compute_option_hash(option).to_vec()
    }

    fn parsed() -> Vec<pb::NewsletterMessage> {
        let node = response();
        parse_history(&node.as_node_ref(), CHANNEL)
    }

    // The point of issue #26: a history row must land in the same shape the
    // event bus delivers, so the edge holds one decoder, not two.
    #[test]
    fn a_history_row_projects_like_a_bus_message() {
        let rows = parsed();
        assert_eq!(rows.len(), 2);
        let message = rows[0].message.clone().expect("a row carries its message");
        assert_eq!(message.text, "bom dia");
        assert_eq!(message.chat, CHANNEL);
        let key = message.key.expect("the key is always built");
        assert_eq!(key.remote_jid, CHANNEL);
        assert_eq!(key.id, "3AEC8E07F1928E3484F7");
        assert!(!key.from_me);
    }

    // Seconds on the wire, milliseconds in the contract.
    #[test]
    fn the_timestamp_relays_in_milliseconds() {
        let message = parsed()[0].message.clone().expect("a row has its message");
        assert_eq!(message.timestamp, 1_788_795_311_000);
    }

    // The node names no sender, so nothing may be invented from `is_sender`
    // beyond `from_me`.
    #[test]
    fn nothing_but_from_me_is_said_about_the_sender() {
        let message = parsed()[0].message.clone().expect("a row has its message");
        assert!(message.sender.is_empty());
        assert!(message.push_name.is_empty());
        assert!(message.sender_alt.is_empty());
    }

    #[test]
    fn is_sender_becomes_from_me() {
        let mine = NodeBuilder::new("message")
            .attr("server_id", "1")
            .attr("is_sender", "true")
            .build();
        let row = message_node_to_proto(&mine.as_node_ref(), CHANNEL)
            .expect("a server_id is all a row needs");
        assert!(
            row.message
                .expect("addressing survives")
                .key
                .unwrap()
                .from_me
        );
    }

    // The tallies are the reason this RPC exists: the server counts them and
    // they ride on no payload.
    #[test]
    fn the_server_side_tallies_relay() {
        let rows = parsed();
        assert_eq!(rows[0].forwards_count, 8329);
        assert_eq!(rows[0].reactions.len(), 1);
        assert_eq!(rows[0].reactions[0].code, "\u{1F621}");
        assert_eq!(rows[0].reactions[0].count, 437);
        assert_eq!(rows[1].forwards_count, 750);
    }

    // What issue #26 went looking for. The option is named by hash, which is
    // what makes a channel poll readable without any `message_secret`.
    #[test]
    fn a_poll_row_carries_its_vote_tally_by_option_hash() {
        let poll = &parsed()[1];
        assert_eq!(poll.r#type, "poll");
        assert_eq!(poll.poll_type, "creation");
        // The vote with no bytes names no option and is dropped.
        assert_eq!(poll.votes.len(), 1);
        assert_eq!(poll.votes[0].count, 25328);
        assert_eq!(poll.votes[0].option_hash, option_hash(POLL_OPTION));
        assert_eq!(poll.votes[0].option_hash.len(), 32);
    }

    // `poll` is not in the library's own enum. Relaying the attribute verbatim
    // is the only reason it survives.
    #[test]
    fn an_unknown_type_relays_verbatim() {
        let odd = NodeBuilder::new("message")
            .attr("server_id", "9")
            .attr("type", "something_new")
            .build();
        let row = message_node_to_proto(&odd.as_node_ref(), CHANNEL).expect("server_id is enough");
        assert_eq!(row.r#type, "something_new");
    }

    // `server_id` is the cursor and the key every add-on hangs off. A row
    // without one cannot be addressed, and a 0 would collide with every other.
    #[test]
    fn a_row_without_a_server_id_is_skipped() {
        let anonymous = NodeBuilder::new("message").attr("type", "text").build();
        assert!(message_node_to_proto(&anonymous.as_node_ref(), CHANNEL).is_none());
    }

    // A row whose `<plaintext>` is absent must not gain a payload.
    #[test]
    fn a_row_without_a_payload_stays_at_proto3_defaults() {
        let bare = NodeBuilder::new("message")
            .attr("server_id", "5")
            .attr("t", "1788795311")
            .build();
        let row = message_node_to_proto(&bare.as_node_ref(), CHANNEL).expect("server_id is enough");
        let message = row
            .message
            .expect("the addressing survives an empty payload");
        assert!(message.raw_message.is_empty());
        assert!(message.text.is_empty());
        assert_eq!(message.chat, CHANNEL);
    }

    // `raw_message` is what an advanced consumer decodes.
    #[test]
    fn raw_message_carries_the_payload_it_projected() {
        let raw = parsed()[0]
            .message
            .clone()
            .expect("a row has its message")
            .raw_message;
        let decoded = wa::Message::decode(&mut raw.as_slice()).expect("the server encoded it");
        assert_eq!(decoded.conversation.as_deref(), Some("bom dia"));
    }

    // An IQ that succeeded but holds no `<messages>` is a channel with no
    // history, not a failure.
    #[test]
    fn a_response_without_messages_is_an_empty_page() {
        let empty = NodeBuilder::new("iq").attr("type", "result").build();
        assert!(parse_history(&empty.as_node_ref(), CHANNEL).is_empty());
    }

    // A zero page size asks the server for nothing; it must read as the
    // caller's mistake, not as an empty channel.
    #[test]
    fn a_zero_count_is_an_invalid_argument() {
        let err = require_count(0).expect_err("0 must be refused");
        assert!(matches!(err, WamuxError::InvalidArgument(_)), "{err}");
        assert!(require_count(1).is_ok());
    }

    // The addressing is the half that was silently wrong: the library sends
    // `to = <channel>` with a bare `<messages count/>` and the server answers
    // nothing at all. See the workaround note above `get_messages`.
    #[test]
    fn the_query_is_addressed_to_the_server_and_names_the_channel_inside() {
        let jid: Jid = CHANNEL.parse().expect("a valid channel jid");
        let query = history_query(&jid, 20, Some(777)).expect("the server jid parses");
        assert_eq!(query.to.to_string(), SERVER_JID);
        let Some(NodeContent::Nodes(children)) = query.content else {
            panic!("the query carries a <messages> child");
        };
        let messages = children[0].as_node_ref();
        assert_eq!(messages.tag.as_ref(), "messages");
        assert_eq!(attr_str(&messages, "type").as_deref(), Some("jid"));
        assert_eq!(attr_str(&messages, "jid").as_deref(), Some(CHANNEL));
        assert_eq!(attr_u64(&messages, "count"), Some(20));
        assert_eq!(attr_u64(&messages, "before"), Some(777));
    }

    // 0 means "newest", so it must not travel as `before="0"`.
    #[test]
    fn a_zero_cursor_leaves_before_off_the_query() {
        let jid: Jid = CHANNEL.parse().expect("a valid channel jid");
        let query = history_query(&jid, 5, None).expect("the server jid parses");
        let Some(NodeContent::Nodes(children)) = query.content else {
            panic!("the query carries a <messages> child");
        };
        assert!(attr_str(&children[0].as_node_ref(), "before").is_none());
    }
}
