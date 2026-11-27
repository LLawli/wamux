//! SendMedia with a sound sticker (#278): the sticker image and its clip are
//! two uploads, streamed in any order, and the message the peer opens is the
//! sticker inside `audioStickerMessage`, carrying its clip. Each upload is
//! proved by value, the way `media.rs` proves one: the plaintext, encrypted
//! again with the key the peer opened, is the body the CDN received.

use std::time::Duration;

use tonic::Code;
use wacore::download::MediaType;
use wacore_binary::Node;
use wamux::proto::v1 as pb;
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::waproto::whatsapp::message::sticker_message::Audio;

use crate::common::mock_wire::{attr, sent_message};
use crate::messaging_service::harness::{
    Fixture, MEDIA_LIMIT, assert_only_the_probe_was_sent, fixture, sender, wire_count,
};
use crate::messaging_service::media::{chunk, head, header, plaintext, send_media};

const CLIP_MIME: &str = "audio/ogg; codecs=opus";
const PATIENCE: Duration = Duration::from_secs(10);

/// A STICKER header that announces a 3-second clip.
fn sound_sticker_header(f: &Fixture) -> pb::SendMediaHeader {
    pb::SendMediaHeader {
        mime_type: "image/webp".into(),
        caption: String::new(),
        sticker_audio: Some(pb::StickerAudio {
            mime_type: CLIP_MIME.into(),
            seconds: 3,
        }),
        ..header(f, pb::MediaType::Sticker)
    }
}

fn clip_part(bytes: &[u8]) -> pb::SendMediaChunk {
    pb::SendMediaChunk {
        part: Some(pb::send_media_chunk::Part::StickerAudio(bytes.to_vec())),
    }
}

/// A clip that differs from the image byte for byte, so the two uploads
/// cannot be confused.
fn clip_plaintext(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 % 241) as u8 ^ 0x5a).collect()
}

fn media_key(key: &Option<Vec<u8>>) -> [u8; 32] {
    key.clone()
        .expect("mediaKey")
        .try_into()
        .expect("a 32-byte media key")
}

/// The sticker and its clip, as the peer opens them.
fn opened_sound_sticker(
    opened: &wa::Message,
) -> (wa::message::StickerMessage, wa::message::AudioMessage) {
    assert!(opened.sticker_message.is_unset(), "the sticker is wrapped");
    let inner = opened
        .audio_sticker_message
        .as_option()
        .expect("audioStickerMessage")
        .message
        .as_option()
        .expect("the wrapped message");
    let sticker = inner
        .sticker_message
        .as_option()
        .expect("stickerMessage")
        .clone();
    let Some(Audio::AudioMessage(clip)) = sticker.audio.clone() else {
        panic!("the sticker carries its clip: {:?}", sticker.audio);
    };
    (sticker, *clip)
}

/// Every `<enc>` the stanza carries, at any depth (a DM to a multi-device
/// peer nests them under `<participants><to>`).
fn encs(node: &Node) -> Vec<&Node> {
    let mut found = Vec::new();
    for child in node.children().unwrap_or_default() {
        if child.tag == "enc" {
            found.push(child);
        }
        found.extend(encs(child));
    }
    found
}

/// Send a sound sticker built of `frames`, then check both uploads and the
/// message the peer opens against the two plaintexts.
async fn assert_sound_sticker_sent(
    f: &mut Fixture,
    frames: Vec<pb::SendMediaChunk>,
    image: &[u8],
    clip: &[u8],
) {
    let sent = send_media(f, frames).await.expect("send a sound sticker");
    let uploads = f.cdn.uploads();
    assert_eq!(uploads.len(), 2, "the image and the clip: {uploads:?}");
    let image_up = uploads
        .iter()
        .find(|u| u.path.starts_with("/mms/image"))
        .expect("the sticker went up as an image");
    let clip_up = uploads
        .iter()
        .find(|u| u.path.starts_with("/mms/audio"))
        .expect("the clip went up as audio");

    let stanza = sent_message(&f.mock, &sent.key.expect("key").id).await;
    assert_eq!(attr(&stanza, "type").as_deref(), Some("media"));
    let tagged = encs(&stanza);
    assert!(!tagged.is_empty(), "the stanza carries an <enc>");
    for enc in tagged {
        assert_eq!(
            attr(enc, "mediatype").as_deref(),
            Some("sticker"),
            "the server drops a media stanza with no mediatype"
        );
    }

    let opened = f.peer.open_dm(&stanza, &sender()).await.expect("open");
    let (sticker, audio) = opened_sound_sticker(&opened);
    assert_eq!(sticker.mimetype.as_deref(), Some("image/webp"));
    assert_eq!(
        sticker.direct_path.as_deref(),
        Some(image_up.direct_path.as_str())
    );
    assert_eq!(sticker.file_length, Some(image.len() as u64));
    let again = wacore::upload::encrypt_media_with_key(
        image,
        MediaType::Sticker,
        Some(&media_key(&sticker.media_key)),
    )
    .expect("encrypt the image again");
    assert_eq!(sticker.file_sha256.as_deref(), Some(&again.file_sha256[..]));
    assert_eq!(image_up.body, again.data_to_upload, "the CDN got the image");

    assert_eq!(audio.mimetype.as_deref(), Some(CLIP_MIME));
    assert_eq!(audio.seconds, Some(3));
    assert_eq!(audio.ptt, None);
    assert_eq!(
        audio.direct_path.as_deref(),
        Some(clip_up.direct_path.as_str())
    );
    assert_eq!(audio.file_length, Some(clip.len() as u64));
    assert!(
        audio.streaming_sidecar.is_some(),
        "the clip plays only with it (#18)"
    );
    let again = wacore::upload::encrypt_media_with_key(
        clip,
        MediaType::Audio,
        Some(&media_key(&audio.media_key)),
    )
    .expect("encrypt the clip again");
    assert_eq!(audio.file_sha256.as_deref(), Some(&again.file_sha256[..]));
    assert_eq!(clip_up.body, again.data_to_upload, "the CDN got the clip");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_media_sound_sticker_uploads_both_and_wraps_the_sticker() {
    let mut f = fixture("send_media_sound_sticker_uploads_both_and_wraps_the_sticker").await;
    let image = plaintext(30_000);
    let clip = clip_plaintext(70_000);
    let frames = vec![
        head(sound_sticker_header(&f)),
        chunk(&image),
        clip_part(&clip[..40_000]),
        clip_part(&clip[40_000..]),
    ];
    assert_sound_sticker_sent(&mut f, frames, &image, &clip).await;
    f.cleanup().await;
}

// The edge streams each part as it produces it: the clip may come first and
// the two may alternate.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_media_sound_sticker_parts_in_any_order() {
    let mut f = fixture("send_media_sound_sticker_parts_in_any_order").await;
    let image = plaintext(30_000);
    let clip = clip_plaintext(70_000);
    let frames = vec![
        head(sound_sticker_header(&f)),
        clip_part(&clip[..10_000]),
        chunk(&image[..12_000]),
        clip_part(&clip[10_000..50_000]),
        chunk(&image[12_000..]),
        clip_part(&clip[50_000..]),
    ];
    assert_sound_sticker_sent(&mut f, frames, &image, &clip).await;
    f.cleanup().await;
}

// Each refusal is the caller's mistake and ends the call before anything is
// sent; none of them leaves an upload on the CDN either.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_media_sound_sticker_refusals_are_invalid_argument() {
    let mut f = fixture("send_media_sound_sticker_refusals_are_invalid_argument").await;
    let before = wire_count(&f);
    let image = plaintext(5_000);
    let clip = clip_plaintext(5_000);
    let on_an_image = pb::SendMediaHeader {
        media_type: pb::MediaType::Image as i32,
        ..sound_sticker_header(&f)
    };
    let plain_sticker = pb::SendMediaHeader {
        sticker_audio: None,
        ..sound_sticker_header(&f)
    };
    let cases = [
        (
            "sticker_audio on an image",
            vec![head(on_an_image), chunk(&image), clip_part(&clip)],
            "MEDIA_TYPE_STICKER",
        ),
        (
            "a clip part without sticker_audio in the header",
            vec![head(plain_sticker), chunk(&image), clip_part(&clip)],
            "sticker_audio",
        ),
        (
            "sticker_audio in the header and no clip bytes",
            vec![head(sound_sticker_header(&f)), chunk(&image)],
            "sticker_audio",
        ),
    ];
    for (case, frames, named) in cases {
        let status = send_media(&mut f, frames).await.expect_err(case);
        assert_eq!(status.code(), Code::InvalidArgument, "{case}: {status:?}");
        assert!(status.message().contains(named), "{case}: {status:?}");
    }
    assert_eq!(f.cdn.uploads().len(), 0, "nothing reached the CDN");
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}

// The limit holds per file: a clip one byte over it is refused as it streams,
// with nothing sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_media_sound_sticker_clip_over_the_limit_is_resource_exhausted() {
    let mut f = fixture("send_media_sound_sticker_clip_over_the_limit_is_resource_exhausted").await;
    let before = wire_count(&f);
    let image = plaintext(MEDIA_LIMIT as usize - 10);
    let clip = clip_plaintext(MEDIA_LIMIT as usize + 1);
    let frames = vec![
        head(sound_sticker_header(&f)),
        chunk(&image),
        clip_part(&clip),
    ];
    let status = send_media(&mut f, frames)
        .await
        .expect_err("a clip over the limit");
    assert_eq!(status.code(), Code::ResourceExhausted, "{status:?}");
    assert_only_the_probe_was_sent(&mut f, before).await;
    f.cleanup().await;
}

// The image and the clip together may pass the limit: each is its own upload.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_media_sound_sticker_limit_holds_per_file() {
    let mut f = fixture("send_media_sound_sticker_limit_holds_per_file").await;
    let image = plaintext(MEDIA_LIMIT as usize - 10);
    let clip = clip_plaintext(MEDIA_LIMIT as usize - 10);
    let frames = vec![
        head(sound_sticker_header(&f)),
        chunk(&image),
        clip_part(&clip),
    ];
    assert_sound_sticker_sent(&mut f, frames, &image, &clip).await;
    f.cleanup().await;
}

/// The next echo of this account's own send, skipping other events.
async fn next_echo(stream: &mut tonic::Streaming<pb::EventEnvelope>) -> pb::InboundMessage {
    loop {
        let envelope = tokio::time::timeout(PATIENCE, stream.message())
            .await
            .expect("an event in time")
            .expect("the stream is healthy")
            .expect("the stream is open");
        let Some(pb::event_envelope::Event::Message(message)) = envelope.event else {
            continue;
        };
        if message.key.as_ref().is_some_and(|key| key.from_me) {
            return message;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_media_sound_sticker_echo_carries_the_clip() {
    let mut f = fixture("send_media_sound_sticker_echo_carries_the_clip").await;
    let request = pb::SubscribeRequest {
        selector: Some(pb::subscribe_request::Selector::Account(f.account.clone())),
        replay_from_ring: 0,
    };
    let mut events = f
        .events
        .clone()
        .subscribe_events(request)
        .await
        .expect("subscribe")
        .into_inner();
    let image = plaintext(20_000);
    let clip = clip_plaintext(20_000);
    let frames = vec![
        head(sound_sticker_header(&f)),
        chunk(&image),
        clip_part(&clip),
    ];
    send_media(&mut f, frames)
        .await
        .expect("send a sound sticker");
    let clip_up = f
        .cdn
        .uploads()
        .into_iter()
        .find(|u| u.path.starts_with("/mms/audio"))
        .expect("the clip went up");

    let echo = next_echo(&mut events).await;
    let media = echo.media.expect("the sticker image");
    assert_eq!(media.media_type(), pb::MediaType::Sticker);
    let audio = echo.sticker_audio.expect("the echo carries the clip");
    assert_eq!(audio.media_type(), pb::MediaType::Audio);
    assert_eq!(audio.direct_path, clip_up.direct_path);
    assert_eq!(audio.mime_type, CLIP_MIME);
    assert_eq!(audio.file_length, clip.len() as u64);
    f.cleanup().await;
}

// Without `sticker_audio` a sticker is exactly what it was before #278.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plain_sticker_is_still_a_sticker_message() {
    let mut f = fixture("a_plain_sticker_is_still_a_sticker_message").await;
    let image = plaintext(20_000);
    let plain = pb::SendMediaHeader {
        sticker_audio: None,
        ..sound_sticker_header(&f)
    };
    let sent = send_media(&mut f, vec![head(plain), chunk(&image)])
        .await
        .expect("send a sticker");
    assert_eq!(f.cdn.uploads().len(), 1, "one upload");
    let stanza = sent_message(&f.mock, &sent.key.expect("key").id).await;
    let opened = f.peer.open_dm(&stanza, &sender()).await.expect("open");
    assert!(opened.audio_sticker_message.is_unset(), "no wrapper");
    let sticker = opened.sticker_message.as_option().expect("stickerMessage");
    assert_eq!(sticker.audio, None, "no clip");
    assert_eq!(sticker.file_length, Some(image.len() as u64));
    f.cleanup().await;
}
