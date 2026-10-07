//! The library's `NewsletterMetadata` onto the wire shape (#56). The wire side
//! (what the server sends, what the library parses it into) is covered through
//! a real client in `tests/stress_newsletter_parse.rs`.

use super::*;
use whatsapp_rust::features::{NewsletterRole, NewsletterState, NewsletterVerification};

/// A jid as the core relays it out (#121): the value verbatim.
fn wire(value: &str) -> Option<pb::Jid> {
    Some(pb::Jid {
        value: value.to_string(),
    })
}

const CHANNEL: &str = "120363144038483540@newsletter";

/// The live WhatsApp channel, as the library parses the answer captured from
/// the production accounts.
fn metadata() -> NewsletterMetadata {
    NewsletterMetadata {
        jid: CHANNEL.parse().expect("channel jid"),
        name: "WhatsApp".into(),
        description: Some("WhatsApp's official channel.".into()),
        subscriber_count: 4242,
        verification: NewsletterVerification::Verified,
        state: NewsletterState::Active,
        picture_url: None,
        preview_url: Some("/v/t61.24694-24/416962407.jpg".into()),
        invite_code: Some("0029Va4K0PZ5a245NkngBA2M".into()),
        role: Some(NewsletterRole::Subscriber),
        creation_time: Some(1_688_746_895),
        muted: None,
        follower_activity_muted: None,
    }
}

// The whole point of issue #6: the name has to survive the projection.
#[test]
fn a_channel_relays_its_name_and_jid() {
    let out = metadata_to_proto(&metadata());
    assert_eq!(out.jid, wire(CHANNEL));
    assert_eq!(out.name, "WhatsApp");
    assert_eq!(out.description, "WhatsApp's official channel.");
    assert_eq!(out.subscriber_count, 4242);
    assert_eq!(out.creation_time, 1_688_746_895);
}

// #128: enums, with the raw halves empty on a named value.
#[test]
fn known_variants_relay_as_enum_values() {
    let out = metadata_to_proto(&metadata());
    assert_eq!(out.verification(), pb::NewsletterVerification::Verified);
    assert_eq!(out.state(), pb::NewsletterState::Active);
    assert_eq!(out.role(), pb::NewsletterRole::Subscriber);
    assert_eq!(
        (out.verification_raw.as_str(), out.state_raw.as_str()),
        ("", "")
    );
}

// Upstream #1557 keeps a value it does not model in the server's spelling;
// since #128 the contract carries it verbatim next to UNKNOWN (it used to
// lowercase it).
#[test]
fn an_unmodelled_state_or_verification_is_unknown_verbatim() {
    let mut meta = metadata();
    meta.state = NewsletterState::Other("DELETED".into());
    meta.verification = NewsletterVerification::Other("PENDING_REVIEW".into());
    let out = metadata_to_proto(&meta);
    assert_eq!(out.state(), pb::NewsletterState::Unknown);
    assert_eq!(out.state_raw, "DELETED");
    assert_eq!(out.verification(), pb::NewsletterVerification::Unknown);
    assert_eq!(out.verification_raw, "PENDING_REVIEW");
}

// Accepted loss (see the note in newsletters.rs): no role, or one the library
// has no variant for, is the same `None`, which is UNSPECIFIED.
#[test]
fn no_role_relays_as_unspecified() {
    let mut meta = metadata();
    meta.role = None;
    assert_eq!(
        metadata_to_proto(&meta).role(),
        pb::NewsletterRole::Unspecified
    );
}

#[test]
fn picture_falls_back_to_the_preview_path() {
    assert_eq!(
        metadata_to_proto(&metadata()).picture_url,
        "/v/t61.24694-24/416962407.jpg"
    );
    let mut with_picture = metadata();
    with_picture.picture_url = Some("/v/full.jpg".into());
    assert_eq!(metadata_to_proto(&with_picture).picture_url, "/v/full.jpg");
}

#[test]
fn absent_optional_fields_stay_at_proto3_defaults() {
    let mut meta = metadata();
    meta.description = None;
    meta.preview_url = None;
    meta.creation_time = None;
    let out = metadata_to_proto(&meta);
    assert!(out.description.is_empty());
    assert!(out.picture_url.is_empty());
    assert_eq!(out.creation_time, 0);
}

// #56: the library's NotFound is the contract's NotFound, not the Unavailable
// every other library error defaults to. #116: and the message names the
// channel, where "account ... not found" named the wrong thing.
#[test]
fn a_missing_channel_is_not_found() {
    let err = newsletter_err(NewsletterError::NotFound(
        CHANNEL.parse().expect("channel jid"),
    ));
    let status = tonic::Status::from(err);
    assert_eq!(status.code(), tonic::Code::NotFound);
    assert_eq!(status.message(), format!("newsletter {CHANNEL} not found"));
}
