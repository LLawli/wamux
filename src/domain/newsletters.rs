//! Channel (`@newsletter`) metadata, relayed over `client.newsletter()`.
//!
//! Issue #6: a channel's name existed on WhatsApp's side and never crossed the
//! contract. An inbound newsletter message carries an empty `push_name` and a
//! `sender` equal to the newsletter jid, history sync brings no name, and
//! `GroupService` does not cover them — so a consumer could only ever address
//! these rows by jid. Nothing new is implemented against WhatsApp here; the
//! library already asks, the core just never relayed the answer.

use serde_json::json;
use whatsapp_rust::Client;
use whatsapp_rust::buffa::Message as _;
use whatsapp_rust::features::{MexRequest, NewsletterMessage, NewsletterReactionCount};

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

/// A page of a channel's history, newest first (issue #26).
///
/// Plain IQ, nothing decrypts: a channel is not E2E, so the server hands back
/// the payloads itself. It is also the only place the server's own tallies
/// exist — a channel reaction is a server-side node rather than a
/// `ReactionMessage`, so its counts arrive here or nowhere.
pub async fn get_messages(
    client: &Client,
    req: &pb::GetNewsletterMessagesRequest,
) -> Result<pb::NewsletterMessageList, WamuxError> {
    let jid = parse_jid(&req.jid)?;
    require_count(req.count)?;
    // 0 is proto3's "absent": start at the newest rather than before row zero.
    let before = (req.before != 0).then_some(req.before);
    let found = client
        .newsletter()
        .get_messages(jid, req.count, before)
        .await
        .map_err(client_err)?;
    Ok(pb::NewsletterMessageList {
        messages: found
            .iter()
            .map(|message| newsletter_message_to_proto(message, &req.jid))
            .collect(),
    })
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

/// Project one history row onto the wire shape.
fn newsletter_message_to_proto(message: &NewsletterMessage, chat: &str) -> pb::NewsletterMessage {
    pb::NewsletterMessage {
        message: Some(history_row_to_inbound(message, chat)),
        server_id: message.server_id,
        // `as_str()` gives the primary wire token, and an unknown label passes
        // through verbatim rather than being folded into a known one.
        r#type: message.message_type.as_str().to_string(),
        reactions: message
            .reactions
            .iter()
            .map(reaction_count_to_proto)
            .collect(),
    }
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
fn history_row_to_inbound(message: &NewsletterMessage, chat: &str) -> pb::InboundMessage {
    let mut out = pb::InboundMessage {
        key: Some(pb::MessageKey {
            remote_jid: chat.to_string(),
            id: message.message_id.clone(),
            from_me: message.is_sender,
            participant: String::new(),
        }),
        chat: chat.to_string(),
        timestamp: millis_from_seconds(message.timestamp),
        raw_message: message
            .message
            .as_ref()
            .map(|payload| payload.encode_to_vec())
            .unwrap_or_default(),
        ..Default::default()
    };
    if let Some(payload) = &message.message {
        project_content(&mut out, payload, chat);
    }
    out
}

/// The stanza counts in seconds; every timestamp in this contract is
/// milliseconds. Saturating, so a nonsense value cannot come back as a date in
/// the past.
fn millis_from_seconds(seconds: u64) -> i64 {
    i64::try_from(seconds)
        .unwrap_or(i64::MAX)
        .saturating_mul(1000)
}

fn reaction_count_to_proto(reaction: &NewsletterReactionCount) -> pb::NewsletterReactionCount {
    pb::NewsletterReactionCount {
        code: reaction.code.clone(),
        count: reaction.count,
    }
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
    use whatsapp_rust::features::NewsletterMessageType;
    use whatsapp_rust::waproto::whatsapp as wa;

    const CHANNEL: &str = "120363144038483540@newsletter";

    /// One row shaped the way `parse_newsletter_messages_response` builds it
    /// from `<message server_id=.. t=.. type=.. is_sender=..><plaintext/></>`.
    fn row() -> NewsletterMessage {
        NewsletterMessage {
            message_id: "3EB0141D233AFFAFF9E0".to_string(),
            server_id: 137,
            timestamp: 1_756_400_000,
            message_type: NewsletterMessageType::Text,
            is_sender: false,
            message: Some(wa::Message {
                conversation: Some("bom dia".to_string()),
                ..Default::default()
            }),
            reactions: vec![NewsletterReactionCount {
                code: "👍".to_string(),
                count: 42,
            }],
        }
    }

    // The point of issue #26: a history row must land in the same shape the
    // event bus delivers, so the edge holds one decoder, not two.
    #[test]
    fn a_history_row_projects_like_a_bus_message() {
        let out = newsletter_message_to_proto(&row(), CHANNEL);
        let message = out.message.expect("a row always carries its message");
        assert_eq!(message.text, "bom dia");
        assert_eq!(message.chat, CHANNEL);
        let key = message.key.expect("the key is always built");
        assert_eq!(key.remote_jid, CHANNEL);
        assert_eq!(key.id, "3EB0141D233AFFAFF9E0");
        assert!(!key.from_me);
    }

    // Seconds on the wire, milliseconds in the contract.
    #[test]
    fn the_timestamp_relays_in_milliseconds() {
        let out = newsletter_message_to_proto(&row(), CHANNEL);
        let message = out.message.expect("a row always carries its message");
        assert_eq!(message.timestamp, 1_756_400_000_000);
    }

    // `is_sender` is the only sender information the node has.
    #[test]
    fn is_sender_becomes_from_me_and_nothing_else_is_invented() {
        let mut mine = row();
        mine.is_sender = true;
        let out = newsletter_message_to_proto(&mine, CHANNEL);
        let message = out.message.expect("a row always carries its message");
        assert!(message.key.expect("the key is always built").from_me);
        // The node names no sender, so these stay at the proto3 default.
        assert!(message.sender.is_empty());
        assert!(message.push_name.is_empty());
        assert!(message.sender_alt.is_empty());
    }

    // The server's tallies are the reason this RPC exists: they arrive here or
    // nowhere, since a channel reaction is a server node, not a message.
    #[test]
    fn server_id_and_reaction_counts_relay() {
        let out = newsletter_message_to_proto(&row(), CHANNEL);
        assert_eq!(out.server_id, 137);
        assert_eq!(out.reactions.len(), 1);
        assert_eq!(out.reactions[0].code, "👍");
        assert_eq!(out.reactions[0].count, 42);
    }

    // Same convention as ReceiptEvent.type: a label this core has never seen
    // relays verbatim instead of being folded into a known token.
    #[test]
    fn the_type_relays_as_its_lowercase_wire_token() {
        let mut vote = row();
        vote.message_type = NewsletterMessageType::PollVote;
        assert_eq!(
            newsletter_message_to_proto(&vote, CHANNEL).r#type,
            "poll_vote"
        );

        let mut unknown = row();
        unknown.message_type = NewsletterMessageType::from("something_new");
        assert_eq!(
            newsletter_message_to_proto(&unknown, CHANNEL).r#type,
            "something_new"
        );
    }

    // A row whose `<plaintext>` did not decode must not gain a payload.
    #[test]
    fn a_row_without_a_payload_stays_at_proto3_defaults() {
        let mut bare = row();
        bare.message = None;
        let out = newsletter_message_to_proto(&bare, CHANNEL);
        let message = out
            .message
            .expect("the addressing survives an empty payload");
        assert!(message.raw_message.is_empty());
        assert!(message.text.is_empty());
        assert_eq!(message.chat, CHANNEL);
    }

    // `raw_message` is what an advanced consumer decodes; a text row still
    // carries the whole payload.
    #[test]
    fn raw_message_carries_the_payload_it_projected() {
        let out = newsletter_message_to_proto(&row(), CHANNEL);
        let raw = out
            .message
            .expect("a row always carries its message")
            .raw_message;
        let decoded = wa::Message::decode(&mut raw.as_slice()).expect("we just encoded it");
        assert_eq!(decoded.conversation.as_deref(), Some("bom dia"));
    }

    // A zero page size asks the server for nothing; it must read as the
    // caller's mistake, not as an empty channel.
    #[test]
    fn a_zero_count_is_an_invalid_argument() {
        let err = require_count(0).expect_err("0 must be refused");
        assert!(matches!(err, WamuxError::InvalidArgument(_)), "{err}");
        assert!(require_count(1).is_ok());
    }
}
