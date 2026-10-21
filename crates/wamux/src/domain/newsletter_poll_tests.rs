//! `newsletter_poll` (#205): the message a channel poll publishes, against
//! what the server stored for one.

use wamux_types::NewPoll;
use whatsapp_rust::buffa::Message as _;
use whatsapp_rust::waproto::whatsapp as wa;

use super::{POLL_SECRET_LEN, newsletter_poll_message};
use crate::error::WamuxError;

/// The secret the RPC answered with when the live check published the poll
/// below (base64 `6QxJkmi95TFZ0iWgw3kuFUD3tzDTzFiiMsEePtUhv+4=`).
const CAPTURED_SECRET: [u8; POLL_SECRET_LEN] = [
    0xe9, 0x0c, 0x49, 0x92, 0x68, 0xbd, 0xe5, 0x31, 0x59, 0xd2, 0x25, 0xa0, 0xc3, 0x79, 0x2e, 0x15,
    0x40, 0xf7, 0xb7, 0x30, 0xd3, 0xcc, 0x58, 0xa2, 0x32, 0xc1, 0x1e, 0x3e, 0xd5, 0x21, 0xbf, 0xee,
];

/// The message the server handed back from the channel's history after
/// `SendPoll` published it on whatsapp-rust 6f07e3ab (2026-10-21, account
/// live-trabalho, channel "wamux 205 teste", server id 100): field 35
/// (`messageContextInfo.messageSecret`) and field 64 (`pollCreationMessageV3`).
const CAPTURED_BYTES_B64: &str = "mgIiGiDpDEmSaL3lMVnSJaDDeS4VQPe3MNPMWKIywR4+1SG/7oIEMBIbd2FtdXggMjA1OiBlbnF1ZXRlIGVtIGNhbmFsGgUKA3NpbRoGCgRuw6NvIAEwAQ==";

fn poll(options: &[&str], selectable_count: u32) -> NewPoll {
    NewPoll {
        name: "wamux 205: enquete em canal".to_string(),
        options: options.iter().map(|o| o.to_string()).collect(),
        selectable_count,
    }
}

fn captured_bytes() -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(CAPTURED_BYTES_B64)
        .expect("the capture is valid base64")
}

#[test]
fn newsletter_poll_message_matches_what_the_server_stored() {
    let message =
        newsletter_poll_message(&poll(&["sim", "não"], 1), &CAPTURED_SECRET).expect("a valid poll");
    assert_eq!(message.encode_to_vec(), captured_bytes());
}

#[test]
fn newsletter_poll_message_is_v1_for_more_than_one_choice() {
    let message = newsletter_poll_message(&poll(&["sim", "não", "talvez"], 2), &CAPTURED_SECRET)
        .expect("a valid poll");
    assert!(message.poll_creation_message_v3.as_option().is_none());
    let created = message
        .poll_creation_message
        .as_option()
        .expect("a multi-choice poll is v1");
    assert_eq!(created.name.as_deref(), Some("wamux 205: enquete em canal"));
    let names: Vec<_> = created
        .options
        .iter()
        .map(|o| o.option_name.clone())
        .collect();
    assert_eq!(
        names,
        [
            Some("sim".into()),
            Some("não".into()),
            Some("talvez".into())
        ]
    );
    assert!(created.options.iter().all(|o| o.option_hash.is_none()));
    assert_eq!(created.selectable_options_count, Some(2));
    assert_eq!(
        created.poll_content_type,
        Some(wa::message::PollContentType::TEXT)
    );
    assert_eq!(created.poll_type, None);
    let context = message
        .message_context_info
        .as_option()
        .expect("context info");
    assert_eq!(
        context.message_secret.as_deref(),
        Some(&CAPTURED_SECRET[..])
    );
}

/// The library's own refusals, word for word (`PollError::InvalidPoll`'s
/// "invalid poll: {reason}"), so the edge reads one message for a chat and a
/// channel alike.
#[test]
fn newsletter_poll_message_refuses_what_the_library_refuses() {
    let thirteen: Vec<String> = (0..13).map(|n| format!("opcao {n}")).collect();
    let thirteen: Vec<&str> = thirteen.iter().map(String::as_str).collect();
    let cases = [
        (
            poll(&["sim"], 1),
            "invalid poll: poll must have at least 2 options",
        ),
        (
            poll(&thirteen, 1),
            "invalid poll: polls can have a maximum of 12 options",
        ),
        (
            poll(&["sim", "não"], 0),
            "invalid poll: selectable_count must be between 1 and 2 (got 0)",
        ),
        (
            poll(&["sim", "não"], 3),
            "invalid poll: selectable_count must be between 1 and 2 (got 3)",
        ),
        (
            poll(&["sim", "sim"], 1),
            "invalid poll: duplicate option name: sim",
        ),
    ];
    for (request, expected) in cases {
        match newsletter_poll_message(&request, &CAPTURED_SECRET) {
            Err(WamuxError::InvalidArgument(message)) => assert_eq!(message, expected),
            other => panic!("{expected}: got {other:?}"),
        }
    }
}
