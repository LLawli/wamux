//! Channel (`@newsletter`) metadata, relayed over `client.newsletter()`.
//!
//! Issue #6: a channel's name existed on WhatsApp's side and never crossed the
//! contract. An inbound newsletter message carries an empty `push_name` and a
//! `sender` equal to the newsletter jid, history sync brings no name, and
//! `GroupService` does not cover them — so a consumer could only ever address
//! these rows by jid. Nothing new is implemented against WhatsApp here; the
//! library already asks, the core just never relayed the answer.

use whatsapp_rust::Client;
use whatsapp_rust::features::{
    NewsletterMetadata, NewsletterRole, NewsletterState, NewsletterVerification,
};

use crate::domain::jid_parse::parse_jid;
use crate::error::{WamuxError, client_err};
use crate::proto::v1 as pb;

pub async fn list_subscribed(client: &Client) -> Result<pb::NewsletterList, WamuxError> {
    let found = client
        .newsletter()
        .list_subscribed()
        .await
        .map_err(client_err)?;
    Ok(pb::NewsletterList {
        newsletters: found.into_iter().map(newsletter_to_proto).collect(),
    })
}

pub async fn get_metadata(client: &Client, jid: &str) -> Result<pb::Newsletter, WamuxError> {
    let jid = parse_jid(jid)?;
    let metadata = client
        .newsletter()
        .get_metadata(&jid)
        .await
        .map_err(client_err)?;
    Ok(newsletter_to_proto(metadata))
}

/// Project the library's metadata onto the wire shape. Absent optionals relay as
/// the proto3 default (empty string, zero), the same rule `wire_defaults` pins
/// on the outbound side: the core never substitutes a placeholder for something
/// the server did not say.
fn newsletter_to_proto(metadata: NewsletterMetadata) -> pb::Newsletter {
    pb::Newsletter {
        jid: metadata.jid.to_string(),
        name: metadata.name,
        description: metadata.description.unwrap_or_default(),
        subscriber_count: metadata.subscriber_count,
        picture_url: metadata.picture_url.unwrap_or_default(),
        verification: verification_label(&metadata.verification).to_string(),
        state: state_label(&metadata.state).to_string(),
        role: metadata.role.as_ref().map(role_label).unwrap_or_default(),
        creation_time: metadata.creation_time.unwrap_or(0) as i64,
    }
}

/// Lowercase wire tokens, never Debug casing — the same convention
/// `ReceiptEvent.type` and `CallEvent.action` follow, so the edge codes against
/// the proto rather than against Rust's formatting.
///
/// All three enums are `#[non_exhaustive]`: a variant added upstream relays its
/// lowercased Debug name rather than being folded into a neighbouring token,
/// which would be the core inventing an answer.
fn verification_label(v: &NewsletterVerification) -> String {
    match v {
        NewsletterVerification::Verified => "verified".to_string(),
        NewsletterVerification::Unverified => "unverified".to_string(),
        other => format!("{other:?}").to_lowercase(),
    }
}

fn state_label(s: &NewsletterState) -> String {
    match s {
        NewsletterState::Active => "active".to_string(),
        NewsletterState::Suspended => "suspended".to_string(),
        NewsletterState::Geosuspended => "geosuspended".to_string(),
        other => format!("{other:?}").to_lowercase(),
    }
}

fn role_label(r: &NewsletterRole) -> String {
    match r {
        NewsletterRole::Owner => "owner".to_string(),
        NewsletterRole::Admin => "admin".to_string(),
        NewsletterRole::Subscriber => "subscriber".to_string(),
        NewsletterRole::Guest => "guest".to_string(),
        other => format!("{other:?}").to_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use whatsapp_rust::Jid;

    fn metadata() -> NewsletterMetadata {
        NewsletterMetadata {
            jid: Jid::from_str("120363454900123456@newsletter").unwrap(),
            name: "Canal de Teste".to_string(),
            description: Some("uma descrição".to_string()),
            subscriber_count: 4242,
            verification: NewsletterVerification::Verified,
            state: NewsletterState::Active,
            picture_url: Some("https://mmg.whatsapp.net/pic".to_string()),
            preview_url: None,
            invite_code: None,
            role: Some(NewsletterRole::Subscriber),
            creation_time: Some(1_717_932_000),
        }
    }

    // The whole point of issue #6: the name has to survive the projection.
    #[test]
    fn metadata_relays_the_name_and_the_jid() {
        let out = newsletter_to_proto(metadata());
        assert_eq!(out.jid, "120363454900123456@newsletter");
        assert_eq!(out.name, "Canal de Teste");
        assert_eq!(out.description, "uma descrição");
        assert_eq!(out.subscriber_count, 4242);
        assert_eq!(out.picture_url, "https://mmg.whatsapp.net/pic");
        assert_eq!(out.creation_time, 1_717_932_000);
    }

    // Lowercase tokens per the proto contract, never Debug casing.
    #[test]
    fn enums_relay_as_lowercase_wire_tokens() {
        let out = newsletter_to_proto(metadata());
        assert_eq!(out.verification, "verified");
        assert_eq!(out.state, "active");
        assert_eq!(out.role, "subscriber");
    }

    // A channel the server described sparsely must not gain invented values.
    #[test]
    fn absent_optionals_stay_proto3_defaults() {
        let sparse = NewsletterMetadata {
            description: None,
            picture_url: None,
            role: None,
            creation_time: None,
            ..metadata()
        };
        let out = newsletter_to_proto(sparse);
        assert!(out.description.is_empty());
        assert!(out.picture_url.is_empty());
        assert!(out.role.is_empty());
        assert_eq!(out.creation_time, 0);
        // The name is not optional and must still be there.
        assert_eq!(out.name, "Canal de Teste");
    }
}
