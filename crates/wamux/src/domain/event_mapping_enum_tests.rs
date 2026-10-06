//! #126: the event fields that were free strings or `Debug` output reach the
//! wire as the contract's enums, and a value the library does not name keeps
//! its original token or code. Its own file because `event_mapping_tests.rs` is
//! already past the 500-line rule.

use super::*;
use wacore::types::events::{
    ConnectFailureReason, LoggedOut, Receipt, TempBanReason, TemporaryBan,
};
use wacore::types::message::MessageSource;
use wacore::types::presence::ReceiptType;

use crate::proto::v1::event_envelope::Event as PbEvent;

fn connection_of(event: &Event) -> pb::ConnectionStateChanged {
    match map_event(event).into_iter().next() {
        Some(PbEvent::Connection(c)) => c,
        other => panic!("expected a connection state, got {other:?}"),
    }
}

// A code the library has no variant for used to reach the edge as
// `Unknown(416)` inside `detail`. Now it is UNKNOWN with the server's code.
#[test]
fn a_logout_with_an_unknown_code_relays_the_code() {
    let c = connection_of(&Event::LoggedOut(Box::new(
        LoggedOut::builder()
            .on_connect(true)
            .reason(ConnectFailureReason::Unknown(416))
            .build(),
    )));
    assert_eq!(c.state, pb::ConnectionState::LoggedOut as i32);
    let info = c.logged_out.expect("LOGGED_OUT carries its info");
    assert_eq!(info.reason(), pb::LogoutReason::Unknown);
    assert_eq!(info.reason_code, 416);
}

// Everything the `Debug` of the ban carried, except the raw node, now travels
// typed: the reason, how long it lasts, and the server's message and link.
#[test]
fn a_temporary_ban_carries_reason_expiry_message_and_url() {
    let c = connection_of(&Event::TemporaryBan(Box::new(
        TemporaryBan::builder()
            .code(TempBanReason::SentToTooManyPeople)
            // A duration without a direct chrono dep: the difference of two instants.
            .expire(wacore::time::from_secs(3_600).unwrap() - wacore::time::from_secs(0).unwrap())
            .message("try again later".to_string())
            .url("https://faq.whatsapp.com/ban".to_string())
            .build(),
    )));
    assert_eq!(c.state, pb::ConnectionState::Banned as i32);
    let ban = c.ban.expect("BANNED carries its info");
    assert_eq!(ban.reason(), pb::BanReason::SentToTooManyPeople);
    assert_eq!(ban.reason_code, 0);
    assert_eq!(ban.expire_seconds, 3_600);
    assert_eq!(ban.message, "try again later");
    assert_eq!(ban.url, "https://faq.whatsapp.com/ban");
    assert_eq!(c.logged_out, None);
}

#[test]
fn a_ban_with_an_unknown_code_relays_the_code() {
    let c = connection_of(&Event::TemporaryBan(Box::new(
        TemporaryBan::builder()
            .code(TempBanReason::Unknown(107))
            // A duration without a direct chrono dep: the difference of two instants.
            .expire(wacore::time::from_secs(3_600).unwrap() - wacore::time::from_secs(0).unwrap())
            .build(),
    )));
    let ban = c.ban.expect("BANNED carries its info");
    assert_eq!(
        (ban.reason(), ban.reason_code),
        (pb::BanReason::Unknown, 107)
    );
    // Absent server attributes are empty strings, never a placeholder.
    assert_eq!((ban.message.as_str(), ban.url.as_str()), ("", ""));
}

// The relay never drops what the server said: an unnamed receipt type keeps
// its attribute, beside UNKNOWN.
#[test]
fn a_receipt_with_an_unknown_type_relays_the_token_in_type_raw() {
    let source = MessageSource {
        chat: "120363041234567890@g.us".parse().unwrap(),
        sender: "5511999000111@s.whatsapp.net".parse().unwrap(),
        ..Default::default()
    };
    let event = Event::Receipt(
        Receipt::builder()
            .source(source)
            .message_ids(vec!["AAA111".into()])
            .timestamp(wacore::time::from_secs(1_717_932_111).unwrap())
            .r#type(ReceiptType::Other("future-kind".to_string()))
            .offline(false)
            .build(),
    );
    match map_event(&event).into_iter().next() {
        Some(PbEvent::Receipt(r)) => {
            assert_eq!(r.r#type(), pb::ReceiptType::Unknown);
            assert_eq!(r.type_raw, "future-kind");
        }
        other => panic!("expected a receipt, got {other:?}"),
    }
}
