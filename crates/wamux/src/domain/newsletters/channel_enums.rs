//! A channel's `whatsapp_rust`-typed values as the contract's enums (#128).
//!
//! Matched here, not in wamux-types, because these types live in
//! `whatsapp_rust::features` and wamux-types does not depend on whatsapp-rust
//! (`scripts/check-crate-deps.sh`); the same exception `StickerPackOrigin`
//! took in #126. All four library enums are `#[non_exhaustive]`, so each keeps
//! a fallback arm: a variant added upstream relays as UNKNOWN, never dropped
//! and never guessed. Where the library kept the server's token (`Other`), it
//! relays verbatim next to UNKNOWN.

use whatsapp_rust::features::{
    NewsletterMessageType, NewsletterRole, NewsletterState, NewsletterVerification,
};

use crate::proto::v1 as pb;

/// The channel's state, and the server's spelling when it is UNKNOWN (empty
/// otherwise). The library reads an absent state as `Active` (#56).
pub(crate) fn newsletter_state_of(state: &NewsletterState) -> (pb::NewsletterState, String) {
    let named = match state {
        NewsletterState::Active => pb::NewsletterState::Active,
        NewsletterState::Suspended => pb::NewsletterState::Suspended,
        NewsletterState::Geosuspended => pb::NewsletterState::Geosuspended,
        NewsletterState::Other(raw) => return (pb::NewsletterState::Unknown, raw.clone()),
        // A variant added upstream: UNKNOWN, with no token to keep.
        _ => return (pb::NewsletterState::Unknown, String::new()),
    };
    (named, String::new())
}

/// The verification badge, and the server's spelling when it is UNKNOWN
/// (empty otherwise). The library reads an absent one as `Unverified` (#56).
pub(crate) fn newsletter_verification_of(
    verification: &NewsletterVerification,
) -> (pb::NewsletterVerification, String) {
    let named = match verification {
        NewsletterVerification::Verified => pb::NewsletterVerification::Verified,
        NewsletterVerification::Unverified => pb::NewsletterVerification::Unverified,
        NewsletterVerification::Other(raw) => {
            return (pb::NewsletterVerification::Unknown, raw.clone());
        }
        // A variant added upstream: UNKNOWN, with no token to keep.
        _ => return (pb::NewsletterVerification::Unknown, String::new()),
    };
    (named, String::new())
}

/// This account's role. `None` (the server said nothing, or named a role the
/// library drops) is UNSPECIFIED; there is no token to keep.
pub(crate) fn newsletter_role_of(role: Option<&NewsletterRole>) -> pb::NewsletterRole {
    match role {
        None => pb::NewsletterRole::Unspecified,
        Some(NewsletterRole::Owner) => pb::NewsletterRole::Owner,
        Some(NewsletterRole::Admin) => pb::NewsletterRole::Admin,
        Some(NewsletterRole::Subscriber) => pb::NewsletterRole::Subscriber,
        Some(NewsletterRole::Guest) => pb::NewsletterRole::Guest,
        // A variant added upstream; the library hands no token for a role.
        Some(_) => pb::NewsletterRole::Unknown,
    }
}

/// A history row's `type`, read from the server's token, and the token
/// verbatim when it is UNKNOWN (empty otherwise). `None` (no attribute) is
/// UNSPECIFIED, not the TEXT the library reads absence as (#1558).
pub(crate) fn newsletter_message_type_of(raw: Option<&str>) -> (pb::NewsletterMessageType, String) {
    let Some(token) = raw else {
        return (pb::NewsletterMessageType::Unspecified, String::new());
    };
    // Only the three names the wire produces are named. The library's other
    // five are documented as never produced, so a server sending one is news,
    // and it relays as UNKNOWN with the token (#128).
    match NewsletterMessageType::from(token) {
        NewsletterMessageType::Text => (pb::NewsletterMessageType::Text, String::new()),
        NewsletterMessageType::Media => (pb::NewsletterMessageType::Media, String::new()),
        NewsletterMessageType::Poll => (pb::NewsletterMessageType::Poll, String::new()),
        _ => (pb::NewsletterMessageType::Unknown, token.to_string()),
    }
}
