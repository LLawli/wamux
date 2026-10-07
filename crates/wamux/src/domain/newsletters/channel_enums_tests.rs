//! `channel_enums` (#128): every value the library names, one unknown each,
//! and absence. The server's own spelling survives next to UNKNOWN, verbatim.

use whatsapp_rust::features::{NewsletterRole, NewsletterState, NewsletterVerification};

use super::channel_enums::{
    newsletter_message_type_of, newsletter_role_of, newsletter_state_of, newsletter_verification_of,
};
use crate::proto::v1 as pb;

fn named<T>(value: T) -> (T, String) {
    (value, String::new())
}

#[test]
fn every_newsletter_state_is_named() {
    let states = [
        (NewsletterState::Active, pb::NewsletterState::Active),
        (NewsletterState::Suspended, pb::NewsletterState::Suspended),
        (
            NewsletterState::Geosuspended,
            pb::NewsletterState::Geosuspended,
        ),
    ];
    for (state, expected) in states {
        assert_eq!(newsletter_state_of(&state), named(expected), "{state:?}");
    }
}

// A deleted channel reads `DELETED` (library doc); relayed as spelled, not
// lowercased as it was before #128.
#[test]
fn an_other_state_is_unknown_with_the_servers_spelling() {
    assert_eq!(
        newsletter_state_of(&NewsletterState::Other("DELETED".into())),
        (pb::NewsletterState::Unknown, "DELETED".to_string())
    );
}

#[test]
fn every_verification_is_named() {
    assert_eq!(
        newsletter_verification_of(&NewsletterVerification::Verified),
        named(pb::NewsletterVerification::Verified)
    );
    assert_eq!(
        newsletter_verification_of(&NewsletterVerification::Unverified),
        named(pb::NewsletterVerification::Unverified)
    );
}

#[test]
fn an_other_verification_is_unknown_with_the_servers_spelling() {
    assert_eq!(
        newsletter_verification_of(&NewsletterVerification::Other("PENDING_REVIEW".into())),
        (
            pb::NewsletterVerification::Unknown,
            "PENDING_REVIEW".to_string()
        )
    );
}

#[test]
fn every_role_is_named() {
    let roles = [
        (NewsletterRole::Owner, pb::NewsletterRole::Owner),
        (NewsletterRole::Admin, pb::NewsletterRole::Admin),
        (NewsletterRole::Subscriber, pb::NewsletterRole::Subscriber),
        (NewsletterRole::Guest, pb::NewsletterRole::Guest),
    ];
    for (role, expected) in roles {
        assert_eq!(newsletter_role_of(Some(&role)), expected, "{role:?}");
    }
}

// The library turns a role it does not model into `None`, the same value as
// "the server said nothing": both are UNSPECIFIED, with no token to keep.
#[test]
fn an_absent_or_dropped_role_is_unspecified() {
    assert_eq!(newsletter_role_of(None), pb::NewsletterRole::Unspecified);
}

#[test]
fn every_wire_message_type_is_named() {
    let types = [
        ("text", pb::NewsletterMessageType::Text),
        ("media", pb::NewsletterMessageType::Media),
        ("poll", pb::NewsletterMessageType::Poll),
    ];
    for (token, expected) in types {
        assert_eq!(
            newsletter_message_type_of(Some(token)),
            named(expected),
            "{token}"
        );
    }
}

// `hologram` is a token nobody names; `reaction` is one the library names but
// documents as never produced by the wire, so the contract does not name it
// (#128) and a row carrying it still relays its token. Case is the server's.
#[test]
fn an_unmodelled_message_type_is_unknown_with_its_token() {
    for token in ["hologram", "reaction", "revoke", "TEXT"] {
        assert_eq!(
            newsletter_message_type_of(Some(token)),
            (pb::NewsletterMessageType::Unknown, token.to_string()),
            "{token}"
        );
    }
}

// No `type` attribute: the library reads it as `Text`; the contract says
// nothing (#1558).
#[test]
fn an_absent_message_type_is_unspecified() {
    assert_eq!(
        newsletter_message_type_of(None),
        named(pb::NewsletterMessageType::Unspecified)
    );
}
