//! `StatusText`, `StatusMedia`, `StatusRevoke` (#41, #115). Tests and the
//! mock only: no live test posts a status.

use wamux_proto::v1 as pb;

use crate::{Jid, MediaKind, StatusMedia, StatusRevoke, StatusText, WamuxError};

const PHONE: &str = "5511999000111@s.whatsapp.net";
const LID: &str = "169815004184633@lid";

fn invalid_argument<T: std::fmt::Debug>(result: Result<T, WamuxError>) -> String {
    match result {
        Err(WamuxError::InvalidArgument(message)) => message,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

fn strings(values: &[&str]) -> Vec<pb::Jid> {
    values
        .iter()
        .map(|v| pb::Jid {
            value: v.to_string(),
        })
        .collect()
}

fn jids(values: &[&str]) -> Vec<Jid> {
    values.iter().map(|v| Jid::parse(v).unwrap()).collect()
}

const STATUS: &str = "status media_type must be one of MEDIA_TYPE_IMAGE|MEDIA_TYPE_VIDEO, got";

fn media_header(media_type: pb::MediaType, recipients: &[&str]) -> pb::PostStatusMediaHeader {
    pb::PostStatusMediaHeader {
        media_type: media_type as i32,
        recipients: strings(recipients),
        ..Default::default()
    }
}

fn revoke_request(message_id: &str, recipients: &[&str]) -> pb::RevokeStatusRequest {
    pb::RevokeStatusRequest {
        account: None,
        message_id: message_id.to_string(),
        recipients: strings(recipients),
    }
}

// 0 is a valid transparent background and relays as given; the font is the
// domain's to check.
#[test]
fn status_text_parses_recipients() {
    let status = StatusText::try_from(pb::PostStatusTextRequest {
        account: None,
        text: "bom dia".to_string(),
        background_argb: 0,
        font: 3,
        recipients: strings(&[PHONE, LID]),
    })
    .unwrap();
    assert_eq!(status.text, "bom dia");
    assert_eq!(status.background_argb, 0);
    assert_eq!(status.font, 3);
    assert_eq!(status.recipients, jids(&[PHONE, LID]));
}

#[test]
fn status_text_refuses_a_malformed_recipient() {
    let result = StatusText::try_from(pb::PostStatusTextRequest {
        recipients: strings(&[PHONE, ""]),
        ..Default::default()
    });
    assert_eq!(invalid_argument(result), "missing jid");
}

// An empty recipient list is the library's to refuse (#101, pinned "today"
// over the socket); the conversion does not add a check of its own.
#[test]
fn status_text_leaves_an_empty_recipient_list_to_the_library() {
    let status = StatusText::try_from(pb::PostStatusTextRequest::default()).unwrap();
    assert!(status.recipients.is_empty());
}

#[test]
fn status_media_converts_every_field() {
    let status = StatusMedia::try_from(pb::PostStatusMediaHeader {
        account: None,
        media_type: pb::MediaType::Video as i32,
        mime_type: "video/mp4".to_string(),
        thumbnail: vec![0xff, 0xd8],
        caption: "legenda".to_string(),
        seconds: 12,
        recipients: strings(&[PHONE]),
    })
    .unwrap();
    assert_eq!(status.kind, MediaKind::Video);
    assert_eq!(status.recipients, jids(&[PHONE]));
    assert_eq!(status.caption, "legenda");
    assert_eq!(status.thumbnail, vec![0xff, 0xd8]);
    assert_eq!(status.seconds, 12);
}

// Audio, document and sticker are valid for a message but NOT for a status.
#[test]
fn status_media_refuses_a_non_status_kind() {
    let refused = [
        pb::MediaType::Audio,
        pb::MediaType::Document,
        pb::MediaType::Sticker,
        pb::MediaType::StickerPack,
        pb::MediaType::Unspecified,
        pb::MediaType::Unknown,
    ];
    for bad in refused {
        assert_eq!(
            invalid_argument(StatusMedia::try_from(media_header(bad, &[PHONE]))),
            format!("{STATUS} {}", bad.as_str_name())
        );
    }
}

// The kind is read before the recipients, the order the domain used.
#[test]
fn status_media_refuses_a_malformed_recipient() {
    let message = invalid_argument(StatusMedia::try_from(media_header(
        pb::MediaType::Image,
        &["not a jid"],
    )));
    assert!(message.starts_with("invalid jid 'not a jid'"), "{message}");
    assert_eq!(
        invalid_argument(StatusMedia::try_from(media_header(
            pb::MediaType::Audio,
            &["not a jid"]
        ))),
        format!("{STATUS} MEDIA_TYPE_AUDIO")
    );
}

#[test]
fn status_revoke_keeps_the_id_and_every_recipient() {
    let revoke = StatusRevoke::try_from(revoke_request("3EB0STATUS", &[PHONE, LID])).unwrap();
    assert_eq!(revoke.message_id.as_str(), "3EB0STATUS");
    assert_eq!(revoke.recipients, jids(&[PHONE, LID]));
}

#[test]
fn status_revoke_refusals_keep_their_messages() {
    assert_eq!(
        invalid_argument(StatusRevoke::try_from(revoke_request("", &[PHONE]))),
        "message_id is empty; expected the status's own id (PostStatus* key.id)"
    );
    assert_eq!(
        invalid_argument(StatusRevoke::try_from(revoke_request("3EB0STATUS", &[]))),
        "recipients is empty; expected the device set the status was posted to"
    );
    assert_eq!(
        invalid_argument(StatusRevoke::try_from(revoke_request("3EB0STATUS", &[""]))),
        "missing jid"
    );
}
