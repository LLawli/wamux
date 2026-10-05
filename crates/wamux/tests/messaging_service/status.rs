//! Status posts and their revoke, to the mock only: no live test ever posts a
//! status (`_rules/live-whatsapp-sends.md`). A status is a sender-key message
//! to `status@broadcast`; each recipient opens it with the key distributed to
//! it.

use tonic::Code;
use wacore::download::MediaType;
use wamux::proto::v1 as pb;
use whatsapp_rust::buffa::Enumeration;
use whatsapp_rust::waproto::whatsapp as wa;

use crate::common::mock_wire::{attr, sent_message};
use crate::harness::{
    Fixture, MEDIA_LIMIT, assert_only_the_probe_was_sent, fixture, jid, jids, newest_stanza,
    sender, wire_count,
};
use crate::media::plaintext;

const STATUS: &str = "status@broadcast";

fn recipients(f: &Fixture) -> Vec<pb::Jid> {
    jids(&[f.peer.pn(), f.other.pn()])
}

fn status_text(f: &Fixture, font: i32, recipients: Vec<pb::Jid>) -> pb::PostStatusTextRequest {
    pb::PostStatusTextRequest {
        account: f.a(),
        text: "bom dia, status".into(),
        background_argb: 0xFF11_2233,
        font,
        recipients,
    }
}

/// Post the text status and return its key.
async fn post_text(f: &mut Fixture) -> pb::MessageKey {
    let request = status_text(f, 1, recipients(f));
    f.messages
        .post_status_text(request)
        .await
        .expect("post status")
        .into_inner()
        .key
        .expect("key")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_status_text_reaches_every_recipient() {
    let mut f = fixture("post_status_text_reaches_every_recipient").await;
    let posted = post_text(&mut f).await;
    assert_eq!(posted.chat, jid(STATUS));
    let stanza = sent_message(&f.mock, &posted.id).await;
    assert_eq!(attr(&stanza, "to").as_deref(), Some(STATUS));
    for recipient in [&f.peer, &f.other] {
        let opened = recipient
            .open_group(&stanza, &sender())
            .await
            .expect("open");
        let text = opened.extended_text_message.as_option().expect("extended");
        assert_eq!(text.text.as_deref(), Some("bom dia, status"));
        assert_eq!(text.background_argb, Some(0xFF11_2233));
        assert_eq!(
            text.font,
            wa::message::extended_text_message::FontType::from_i32(1)
        );
    }
    f.cleanup().await;
}

fn status_head(f: &Fixture, media_type: &str) -> pb::PostStatusMediaChunk {
    pb::PostStatusMediaChunk {
        part: Some(pb::post_status_media_chunk::Part::Header(
            pb::PostStatusMediaHeader {
                account: f.a(),
                media_type: media_type.into(),
                mime_type: "image/jpeg".into(),
                caption: "foto do dia".into(),
                recipients: recipients(f),
                ..Default::default()
            },
        )),
    }
}

fn status_chunk(bytes: &[u8]) -> pb::PostStatusMediaChunk {
    pb::PostStatusMediaChunk {
        part: Some(pb::post_status_media_chunk::Part::Chunk(bytes.to_vec())),
    }
}

async fn post_media(
    f: &mut Fixture,
    frames: Vec<pb::PostStatusMediaChunk>,
) -> Result<pb::SendResult, tonic::Status> {
    f.messages
        .post_status_media(tokio_stream::iter(frames))
        .await
        .map(|response| response.into_inner())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_status_media_uploads_and_reaches_every_recipient() {
    let mut f = fixture("post_status_media_uploads_and_reaches_every_recipient").await;
    let plain = plaintext(60_000);
    let frames = vec![status_head(&f, "image"), status_chunk(&plain)];
    let posted = post_media(&mut f, frames).await.expect("post media");
    let uploads = f.cdn.uploads();
    assert_eq!(uploads.len(), 1, "one upload");
    let stanza = sent_message(&f.mock, &posted.key.expect("key").id).await;
    for recipient in [&f.peer, &f.other] {
        let opened = recipient
            .open_group(&stanza, &sender())
            .await
            .expect("open");
        let image = opened.image_message.as_option().expect("imageMessage");
        assert_eq!(image.caption.as_deref(), Some("foto do dia"));
        assert_eq!(
            image.direct_path.as_deref(),
            Some(uploads[0].direct_path.as_str())
        );
        let media_key: [u8; 32] = image
            .media_key
            .clone()
            .expect("mediaKey")
            .try_into()
            .expect("a 32-byte media key");
        let again =
            wacore::upload::encrypt_media_with_key(&plain, MediaType::Image, Some(&media_key))
                .expect("encrypt again");
        assert_eq!(
            uploads[0].body, again.data_to_upload,
            "the CDN got the media"
        );
    }
    f.cleanup().await;
}

// The revoke goes to the same recipients as the status, as a REVOKE naming it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoke_status_reaches_every_recipient() {
    let mut f = fixture("revoke_status_reaches_every_recipient").await;
    let posted = post_text(&mut f).await;
    let status = sent_message(&f.mock, &posted.id).await;
    for recipient in [&f.peer, &f.other] {
        recipient
            .open_group(&status, &sender())
            .await
            .expect("open the status");
    }
    let before = f.mock.client_messages().len();
    let request = pb::RevokeStatusRequest {
        account: f.a(),
        message_id: posted.id.clone(),
        recipients: recipients(&f),
    };
    f.messages.revoke_status(request).await.expect("revoke");
    let stanza = newest_stanza(&f.mock, "message", before).await;
    assert_eq!(attr(&stanza, "to").as_deref(), Some(STATUS));
    for recipient in [&f.peer, &f.other] {
        let opened = recipient
            .open_group(&stanza, &sender())
            .await
            .expect("open");
        let protocol = opened
            .protocol_message
            .as_option()
            .expect("protocolMessage");
        assert_eq!(
            protocol.r#type,
            Some(wa::message::protocol_message::Type::Revoke)
        );
        let target = protocol.key.as_option().expect("key");
        assert_eq!(target.id.as_deref(), Some(posted.id.as_str()));
    }
    f.cleanup().await;
}

/// #101: the library refuses a status with no recipients, which the core
/// relays as Unavailable. Nothing is sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_status_with_no_recipients_is_unavailable_today() {
    let mut f = fixture("a_status_with_no_recipients_is_unavailable_today").await;
    let before = wire_count(&f);
    let request = status_text(&f, 1, Vec::new());
    let status = f
        .messages
        .post_status_text(request)
        .await
        .expect_err("no recipients");
    assert_eq!(status.code(), Code::Unavailable, "{status:?}");
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}

// A font outside wa's enum and a recipient that is not a jid are the caller's
// mistakes: InvalidArgument, nothing sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_status_font_is_invalid_argument() {
    let mut f = fixture("an_unknown_status_font_is_invalid_argument").await;
    let before = wire_count(&f);
    let cases = [
        ("font 999", status_text(&f, 999, recipients(&f))),
        ("a bad recipient", status_text(&f, 1, jids(&["not a jid"]))),
    ];
    for (case, request) in cases {
        let status = f.messages.post_status_text(request).await.expect_err(case);
        assert_eq!(status.code(), Code::InvalidArgument, "{case}: {status:?}");
    }
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}

// Same stream contract as SendMedia, plus a status is an image or a video.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_status_media_stream_errors_are_refused() {
    let mut f = fixture("post_status_media_stream_errors_are_refused").await;
    let before = wire_count(&f);
    let over = plaintext(MEDIA_LIMIT as usize + 1);
    let cases = [
        ("an empty stream", Vec::new(), Code::InvalidArgument),
        (
            "a chunk first",
            vec![status_chunk(b"jpeg"), status_head(&f, "image")],
            Code::InvalidArgument,
        ),
        (
            "a second header",
            vec![
                status_head(&f, "image"),
                status_chunk(b"jpeg"),
                status_head(&f, "image"),
            ],
            Code::InvalidArgument,
        ),
        (
            "an audio status",
            vec![status_head(&f, "audio"), status_chunk(b"ogg")],
            Code::InvalidArgument,
        ),
        (
            "over the limit",
            vec![status_head(&f, "image"), status_chunk(&over)],
            Code::ResourceExhausted,
        ),
    ];
    for (case, frames, code) in cases {
        let status = post_media(&mut f, frames).await.expect_err(case);
        assert_eq!(status.code(), code, "{case}: {status:?}");
    }
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}
