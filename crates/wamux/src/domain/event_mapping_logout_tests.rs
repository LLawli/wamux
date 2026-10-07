//! #134: a forced logout carries the server's own copy (header, subtext and the
//! locale it is written in) and whether the server refused the connection.
//! Both reached the core and were dropped; #126 had left the copy for #74.

use super::*;
use wacore::types::events::{ConnectFailureReason, LogoutMessage};

use crate::proto::v1::event_envelope::Event as PbEvent;

fn logged_out_info(event: &Event) -> pb::LoggedOutInfo {
    match map_event(event).into_iter().next() {
        Some(PbEvent::Connection(c)) => {
            assert_eq!(c.state, pb::ConnectionState::LoggedOut as i32);
            assert_eq!(c.ban, None);
            c.logged_out.expect("LOGGED_OUT carries its info")
        }
        other => panic!("expected a connection change, got {other:?}"),
    }
}

#[test]
fn logged_out_carries_the_logout_message_and_on_connect() {
    let message = LogoutMessage::builder()
        .header("Conta bloqueada".to_string())
        .subtext("Toque para saber mais".to_string())
        .locale("pt_BR".to_string())
        .build();
    let event = Event::LoggedOut(Box::new(
        LoggedOut::builder()
            .on_connect(true)
            .reason(ConnectFailureReason::AccountLocked)
            .logout_message(message)
            .build(),
    ));
    let expected = pb::LoggedOutInfo {
        reason: pb::LogoutReason::AccountLocked as i32,
        reason_code: 0,
        logout_message: Some(pb::LogoutMessage {
            header: Some("Conta bloqueada".to_string()),
            subtext: Some("Toque para saber mais".to_string()),
            locale: Some("pt_BR".to_string()),
        }),
        on_connect: true,
    };
    assert_eq!(logged_out_info(&event), expected);
}

/// No copy from the server is an unset message, never an empty one; a
/// message with only some parts keeps the rest unset.
#[test]
fn logged_out_without_a_message_leaves_it_unset() {
    let event = Event::LoggedOut(Box::new(
        LoggedOut::builder()
            .on_connect(false)
            .reason(ConnectFailureReason::LoggedOut)
            .build(),
    ));
    let info = logged_out_info(&event);
    assert_eq!(info.logout_message, None);
    assert!(!info.on_connect);

    let partial = Event::LoggedOut(Box::new(
        LoggedOut::builder()
            .on_connect(true)
            .reason(ConnectFailureReason::LoggedOut)
            .logout_message(LogoutMessage::builder().header("Saiu".to_string()).build())
            .build(),
    ));
    assert_eq!(
        logged_out_info(&partial).logout_message,
        Some(pb::LogoutMessage {
            header: Some("Saiu".to_string()),
            subtext: None,
            locale: None,
        })
    );
}
