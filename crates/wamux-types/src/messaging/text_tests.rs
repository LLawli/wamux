//! `OutgoingText`, `LinkPreview` (#115).

use wamux_proto::v1 as pb;

use crate::{Jid, LinkPreview, OutgoingText, WamuxError};

fn preview() -> pb::LinkPreview {
    pb::LinkPreview {
        matched_text: "https://example.com/post".to_string(),
        title: "A title".to_string(),
        description: "A description".to_string(),
        jpeg_thumbnail: vec![0xff, 0xd8, 0xff],
        preview_type: 1,
    }
}

#[test]
fn a_link_preview_copies_every_field() {
    assert_eq!(
        LinkPreview::from(preview()),
        LinkPreview {
            matched_text: "https://example.com/post".to_string(),
            title: "A title".to_string(),
            description: "A description".to_string(),
            jpeg_thumbnail: vec![0xff, 0xd8, 0xff],
            preview_type: 1,
        }
    );
}

// An out-of-schema preview_type is the domain's to refuse (it needs the
// library's enum); the conversion carries the number as it came.
#[test]
fn a_link_preview_keeps_an_unknown_type_for_the_domain() {
    let preview = pb::LinkPreview {
        preview_type: 999,
        ..preview()
    };
    assert_eq!(LinkPreview::from(preview).preview_type, 999);
}

// The routing fields are the service's; the content converts whole.
#[test]
fn outgoing_text_carries_text_context_and_preview() {
    let text = OutgoingText::try_from(pb::SendTextRequest {
        account: None,
        to: Some(pb::Jid {
            value: "not read here".to_string(),
        }),
        text: "oi".to_string(),
        mentions: vec![pb::Mention {
            jid: Some(pb::Jid {
                value: "5511888888888@s.whatsapp.net".to_string(),
            }),
        }],
        quote: None,
        link_preview: Some(preview()),
        ephemeral_seconds: 86_400,
    })
    .unwrap();
    assert_eq!(text.text, "oi");
    assert_eq!(
        text.context.mentions,
        vec![Jid::parse("5511888888888@s.whatsapp.net").unwrap()]
    );
    assert_eq!(text.context.ephemeral_seconds, 86_400);
    assert_eq!(text.link_preview, Some(LinkPreview::from(preview())));
}

#[test]
fn a_plain_text_has_no_context_and_no_preview() {
    let text = OutgoingText::try_from(pb::SendTextRequest {
        text: "oi".to_string(),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(text.context, Default::default());
    assert_eq!(text.link_preview, None);
}

#[test]
fn outgoing_text_refuses_a_malformed_mention() {
    let result = OutgoingText::try_from(pb::SendTextRequest {
        mentions: vec![pb::Mention {
            jid: Some(pb::Jid {
                value: "not a jid".to_string(),
            }),
        }],
        ..Default::default()
    });
    assert!(
        matches!(result, Err(WamuxError::InvalidArgument(_))),
        "{result:?}"
    );
}
