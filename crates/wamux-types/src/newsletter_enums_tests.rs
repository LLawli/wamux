//! `poll_type_of` and `edit_attribute_of` (#128): every value the library names,
//! one unknown token each, and absence.

use wacore::types::message::EditAttribute;
use wamux_proto::v1 as pb;

use crate::newsletter_enums::{edit_attribute_of, poll_type_of};

fn named<T>(value: T) -> (T, String) {
    (value, String::new())
}

#[test]
fn every_poll_type_token_is_named() {
    let tokens = [
        ("creation", pb::NewsletterPollType::Creation),
        ("quiz_creation", pb::NewsletterPollType::QuizCreation),
        ("vote", pb::NewsletterPollType::Vote),
        ("result_snapshot", pb::NewsletterPollType::ResultSnapshot),
        ("edit", pb::NewsletterPollType::Edit),
    ];
    for (token, expected) in tokens {
        assert_eq!(poll_type_of(Some(token)), named(expected), "{token}");
    }
}

// The library's `PollType` is closed and drops what it does not know; the
// core keeps the server's token (#128), with its exact spelling.
#[test]
fn an_unlisted_poll_type_is_unknown_with_its_token() {
    for token in ["future_stage", "CREATION", ""] {
        assert_eq!(
            poll_type_of(Some(token)),
            (pb::NewsletterPollType::Unknown, token.to_string()),
            "{token:?}"
        );
    }
}

#[test]
fn an_absent_poll_type_is_unspecified() {
    assert_eq!(
        poll_type_of(None),
        named(pb::NewsletterPollType::Unspecified)
    );
}

#[test]
fn every_edit_attribute_is_named() {
    let named_values = [
        (EditAttribute::MessageEdit, pb::EditAttribute::MessageEdit),
        (EditAttribute::PinInChat, pb::EditAttribute::PinInChat),
        (EditAttribute::AdminEdit, pb::EditAttribute::AdminEdit),
        (EditAttribute::SenderRevoke, pb::EditAttribute::SenderRevoke),
        (EditAttribute::AdminRevoke, pb::EditAttribute::AdminRevoke),
    ];
    for (edit, expected) in named_values {
        assert_eq!(edit_attribute_of(&edit), named(expected), "{edit:?}");
    }
}

// Through the library's own parse, so the server's tokens are what is pinned:
// "3" an admin edit, "8" an admin revoke, "9" a token nobody names yet.
#[test]
fn an_unknown_edit_attribute_keeps_its_token() {
    assert_eq!(
        edit_attribute_of(&EditAttribute::from("3")),
        named(pb::EditAttribute::AdminEdit)
    );
    assert_eq!(
        edit_attribute_of(&EditAttribute::from("8")),
        named(pb::EditAttribute::AdminRevoke)
    );
    assert_eq!(
        edit_attribute_of(&EditAttribute::from("9")),
        (pb::EditAttribute::Unknown, "9".to_string())
    );
}

#[test]
fn an_absent_edit_attribute_is_unspecified() {
    assert_eq!(
        edit_attribute_of(&EditAttribute::Empty),
        named(pb::EditAttribute::Unspecified)
    );
    assert_eq!(
        edit_attribute_of(&EditAttribute::from("")),
        named(pb::EditAttribute::Unspecified)
    );
}
