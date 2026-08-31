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
use whatsapp_rust::features::MexRequest;

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
