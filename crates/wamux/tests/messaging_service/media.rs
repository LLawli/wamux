//! SendMedia: the client stream is read, the media uploaded and its descriptor
//! sent. The upload is proved by value: the plaintext, encrypted again with
//! the media key the peer opened, is exactly the body the CDN received.

use tonic::Code;
use wacore::download::MediaType;
use wamux::proto::v1 as pb;

use crate::common::mock_wire::sent_message;
use crate::harness::{
    Fixture, MEDIA_LIMIT, assert_only_the_probe_was_sent, fixture, jid, sender, wire_count,
};

/// A deterministic plaintext of `len` bytes.
pub fn plaintext(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 % 251) as u8).collect()
}

pub fn header(f: &Fixture, media_type: pb::MediaType) -> pb::SendMediaHeader {
    pb::SendMediaHeader {
        account: f.a(),
        to: jid(f.peer.pn()),
        mime_type: "image/jpeg".into(),
        caption: "uma foto".into(),
        media_type: media_type as i32,
        ..Default::default()
    }
}

pub fn head(header: pb::SendMediaHeader) -> pb::SendMediaChunk {
    pb::SendMediaChunk {
        part: Some(pb::send_media_chunk::Part::Header(header)),
    }
}

pub fn chunk(bytes: &[u8]) -> pb::SendMediaChunk {
    pb::SendMediaChunk {
        part: Some(pb::send_media_chunk::Part::Chunk(bytes.to_vec())),
    }
}

/// Send `frames` as one client stream.
pub async fn send_media(
    f: &mut Fixture,
    frames: Vec<pb::SendMediaChunk>,
) -> Result<pb::SendResult, tonic::Status> {
    f.messages
        .send_media(tokio_stream::iter(frames))
        .await
        .map(|response| response.into_inner())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_media_uploads_and_sends_its_descriptor() {
    let mut f = fixture("send_media_uploads_and_sends_its_descriptor").await;
    let plain = plaintext(100_000);
    let frames = vec![
        head(header(&f, pb::MediaType::Image)),
        chunk(&plain[..40_000]),
        chunk(&plain[40_000..]),
    ];
    let sent = send_media(&mut f, frames).await.expect("send media");
    let uploads = f.cdn.uploads();
    assert_eq!(uploads.len(), 1, "one upload");
    assert!(
        uploads[0].path.starts_with("/mms/image"),
        "{}",
        uploads[0].path
    );

    let stanza = sent_message(&f.mock, &sent.key.expect("key").id).await;
    let opened = f.peer.open_dm(&stanza, &sender()).await.expect("open");
    let image = opened.image_message.as_option().expect("imageMessage");
    assert_eq!(image.mimetype.as_deref(), Some("image/jpeg"));
    assert_eq!(image.caption.as_deref(), Some("uma foto"));
    assert_eq!(
        image.direct_path.as_deref(),
        Some(uploads[0].direct_path.as_str())
    );
    assert_eq!(image.file_length, Some(plain.len() as u64));
    let media_key: [u8; 32] = image
        .media_key
        .clone()
        .expect("mediaKey")
        .try_into()
        .expect("a 32-byte media key");
    let again = wacore::upload::encrypt_media_with_key(&plain, MediaType::Image, Some(&media_key))
        .expect("encrypt again");
    assert_eq!(image.file_sha256.as_deref(), Some(&again.file_sha256[..]));
    assert_eq!(
        image.file_enc_sha256.as_deref(),
        Some(&again.file_enc_sha256[..])
    );
    assert_eq!(
        uploads[0].body, again.data_to_upload,
        "the CDN got the media"
    );
    f.cleanup().await;
}

// The cut-off is checked as the chunks arrive: one byte over the limit ends
// the call before anything is uploaded or sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_media_over_the_limit_is_resource_exhausted() {
    let mut f = fixture("send_media_over_the_limit_is_resource_exhausted").await;
    let before = wire_count(&f);
    let plain = plaintext(MEDIA_LIMIT as usize + 1);
    let frames = vec![head(header(&f, pb::MediaType::Image)), chunk(&plain)];
    let status = send_media(&mut f, frames)
        .await
        .expect_err("over the limit");
    assert_eq!(status.code(), Code::ResourceExhausted, "{status:?}");
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}

// The stream's framing is the caller's: no header, a chunk before the header,
// or a second header are each InvalidArgument, and nothing is uploaded.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_media_stream_framing_errors_are_invalid_argument() {
    let mut f = fixture("send_media_stream_framing_errors_are_invalid_argument").await;
    let before = wire_count(&f);
    let cases = [
        ("an empty stream", Vec::new()),
        (
            "a chunk first",
            vec![chunk(b"jpeg"), head(header(&f, pb::MediaType::Image))],
        ),
        (
            "a second header",
            vec![
                head(header(&f, pb::MediaType::Image)),
                chunk(b"jpeg"),
                head(header(&f, pb::MediaType::Image)),
            ],
        ),
    ];
    for (case, frames) in cases {
        let status = send_media(&mut f, frames).await.expect_err(case);
        assert_eq!(status.code(), Code::InvalidArgument, "{case}: {status:?}");
    }
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}

// `media_type` is one of five sendable values (#127); unset, unknown, a
// download-only kind or a number outside the enum is refused before the
// upload, with the value named.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_media_with_an_unknown_type_is_invalid_argument() {
    let mut f = fixture("send_media_with_an_unknown_type_is_invalid_argument").await;
    let before = wire_count(&f);
    let refused = [
        (pb::MediaType::Unspecified as i32, "MEDIA_TYPE_UNSPECIFIED"),
        (pb::MediaType::Unknown as i32, "MEDIA_TYPE_UNKNOWN"),
        (pb::MediaType::StickerPack as i32, "MEDIA_TYPE_STICKER_PACK"),
        (42, "42"),
    ];
    for (value, shown) in refused {
        let raw = pb::SendMediaHeader {
            media_type: value,
            ..header(&f, pb::MediaType::Image)
        };
        let frames = vec![head(raw), chunk(b"jpeg")];
        let status = send_media(&mut f, frames).await.expect_err(shown);
        assert_eq!(status.code(), Code::InvalidArgument, "{shown}: {status:?}");
        assert!(
            status.message().ends_with(&format!("got {shown}")),
            "{shown}: {status:?}"
        );
    }
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}
