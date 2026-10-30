//! The media spool (#79), through the socket: SendMedia and DownloadMedia stage
//! each transfer in an unlinked file under `media_spool_dir`, a global budget
//! (`media_spool_max_bytes`) bounds what all transfers hold at once, and a
//! download larger than `media_max_bytes` is refused before anything is
//! fetched. The media type is checked before the bytes, since the encryptor
//! needs it to start.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tempfile::TempDir;
use tonic::Code;
use tonic::transport::Channel;
use wacore::download::MediaType;
use wamux::config::Config;
use wamux::proto::v1 as pb;
use wamux::proto::v1::media_service_client::MediaServiceClient;

use crate::common::{self, mock_wire::sent_message};
use crate::messaging_service::harness::{Fixture, fixture_with, jids, sender};
use crate::messaging_service::media::{chunk, head, header, plaintext, send_media};

const KEY: [u8; 32] = [0x79; 32];
/// `media_max_bytes` for these tests.
const LIMIT: u64 = 1024 * 1024;
const KIB: u64 = 1024;

/// The fixture with its own spool directory, `LIMIT` and `budget`.
async fn spool_fixture(test: &str, budget: u64) -> (Fixture, TempDir) {
    let dir = tempfile::tempdir().expect("spool dir");
    let config = Config {
        media_max_bytes: LIMIT,
        media_spool_dir: dir.path().to_str().expect("utf-8").to_string(),
        media_spool_max_bytes: budget,
        ..Config::default()
    };
    (fixture_with(test, config).await, dir)
}

/// Encrypt `plain` as a document with `KEY`, serve it at `path` and return
/// the descriptor an inbound message would carry for it.
fn served(f: &Fixture, plain: &[u8], path: &str) -> pb::MediaDescriptor {
    let enc = wacore::upload::encrypt_media_with_key(plain, MediaType::Document, Some(&KEY))
        .expect("encrypt");
    f.cdn.serve(path, enc.data_to_upload);
    pb::MediaDescriptor {
        direct_path: path.into(),
        media_key: KEY.to_vec(),
        file_enc_sha256: enc.file_enc_sha256.to_vec(),
        file_sha256: enc.file_sha256.to_vec(),
        file_length: plain.len() as u64,
        mime_type: "application/pdf".into(),
        media_type: pb::MediaType::Document as i32,
    }
}

/// Every frame a download delivered, and the status that ended it if not OK.
async fn download_with(
    media: &mut MediaServiceClient<Channel>,
    account: pb::AccountRef,
    descriptor: pb::MediaDescriptor,
) -> (Vec<pb::MediaChunk>, Option<tonic::Status>) {
    let request = pb::DownloadMediaRequest {
        account: Some(account),
        descriptor: Some(descriptor),
    };
    let mut stream = match media.download_media(request).await {
        Ok(response) => response.into_inner(),
        Err(status) => return (Vec::new(), Some(status)),
    };
    let mut frames = Vec::new();
    loop {
        match stream.message().await {
            Ok(Some(frame)) => frames.push(frame),
            Ok(None) => return (frames, None),
            Err(status) => return (frames, Some(status)),
        }
    }
}

async fn download(
    f: &mut Fixture,
    descriptor: pb::MediaDescriptor,
) -> (Vec<pb::MediaChunk>, Option<tonic::Status>) {
    let account = f.account.clone();
    download_with(&mut f.media, account, descriptor).await
}

/// The bytes of a successful download (every chunk after the meta).
fn delivered(frames: &[pb::MediaChunk], status: Option<tonic::Status>) -> Vec<u8> {
    assert!(status.is_none(), "download failed: {status:?}");
    frames[1..]
        .iter()
        .flat_map(|frame| match &frame.part {
            Some(pb::media_chunk::Part::Chunk(bytes)) => bytes.clone(),
            other => panic!("a chunk after the meta, got {other:?}"),
        })
        .collect()
}

fn assert_refused_with(status: Option<tonic::Status>, code: Code, message: &str) {
    let status = status.expect("a refusal");
    assert_eq!(status.code(), code, "{status:?}");
    assert_eq!(status.message(), message, "{status:?}");
}

fn entries(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .expect("read_dir")
        .map(|entry| entry.expect("entry").file_name().to_string_lossy().into())
        .collect()
}

type UploadTask = tokio::task::JoinHandle<Result<tonic::Response<pb::SendResult>, tonic::Status>>;

/// A SendMedia of `len` bytes, running on its own task so the test can watch
/// it from outside and abandon it.
fn spawn_send(f: &Fixture, len: usize) -> UploadTask {
    let frames = vec![
        head(header(f, pb::MediaType::Image)),
        chunk(&plaintext(len)),
    ];
    let mut messages = f.messages.clone();
    tokio::spawn(async move { messages.send_media(tokio_stream::iter(frames)).await })
}

/// Wait until an upload has reached the CDN, which `hold_uploads` keeps from
/// answering. By then the daemon has read and encrypted every byte, so its
/// spool reservation is whole and stays held while the upload is in flight:
/// a fixed point to probe the budget from, with no race against the upload.
async fn until_the_cdn_holds_an_upload(f: &Fixture) {
    common::poll_until(
        "the upload to reach the held CDN",
        Duration::from_secs(10),
        || async { (f.cdn.uploads_waiting() == 1).then_some(()) },
    )
    .await;
}

/// Download `descriptor` until it succeeds, and return its frames.
async fn until_it_downloads(
    f: &Fixture,
    descriptor: &pb::MediaDescriptor,
    what: &str,
) -> Vec<pb::MediaChunk> {
    let (media, account) = (f.media.clone(), f.account.clone());
    common::poll_until(what, Duration::from_secs(10), || {
        let (mut media, account, descriptor) = (media.clone(), account.clone(), descriptor.clone());
        async move {
            let (frames, status) = download_with(&mut media, account, descriptor).await;
            status.is_none().then_some(frames)
        }
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sent_file_uploads_ciphertext_that_decrypts_to_it() {
    let (mut f, dir) = spool_fixture("a_sent_file_uploads_ciphertext_that_decrypts_to_it", 0).await;
    let plain = plaintext(300_000);
    let video = pb::SendMediaHeader {
        mime_type: "video/mp4".into(),
        ..header(&f, pb::MediaType::Video)
    };
    let frames = vec![
        head(video),
        chunk(&plain[..100_000]),
        chunk(&plain[100_000..250_000]),
        chunk(&plain[250_000..]),
    ];
    let sent = send_media(&mut f, frames).await.expect("send media");
    let uploads = f.cdn.uploads();
    assert_eq!(uploads.len(), 1, "one upload");

    let stanza = sent_message(&f.mock, &sent.key.expect("key").id).await;
    let opened = f.peer.open_dm(&stanza, &sender()).await.expect("open");
    let message = opened.video_message.as_option().expect("videoMessage");
    let media_key: [u8; 32] = message
        .media_key
        .clone()
        .expect("mediaKey")
        .try_into()
        .expect("a 32-byte media key");
    let again = wacore::upload::encrypt_media_with_key_and_sidecar(
        &plain,
        MediaType::Video,
        Some(&media_key),
        None,
    )
    .expect("encrypt again");
    assert_eq!(
        uploads[0].body, again.data_to_upload,
        "the CDN got the media"
    );
    assert_eq!(message.file_length, Some(plain.len() as u64));
    assert_eq!(message.file_sha256.as_deref(), Some(&again.file_sha256[..]));
    assert_eq!(
        message.file_enc_sha256.as_deref(),
        Some(&again.file_enc_sha256[..])
    );
    assert!(again.streaming_sidecar.is_some(), "a video has a sidecar");
    assert_eq!(
        message.streaming_sidecar, again.streaming_sidecar,
        "the sidecar survives the spool (#18)"
    );
    assert!(entries(dir.path()).is_empty(), "{:?}", entries(dir.path()));
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_download_larger_than_media_max_bytes_is_refused_before_fetching() {
    let (mut f, _dir) = spool_fixture(
        "a_download_larger_than_media_max_bytes_is_refused_before_fetching",
        0,
    )
    .await;
    let descriptor = pb::MediaDescriptor {
        file_length: LIMIT + 1,
        ..served(&f, &plaintext(1000), "/v/spool/too-big")
    };
    let (frames, status) = download(&mut f, descriptor).await;
    assert!(frames.is_empty(), "no frame: {frames:?}");
    assert_refused_with(status, Code::ResourceExhausted, "media exceeds size limit");
    assert!(
        f.cdn.requests().is_empty(),
        "fetched: {:?}",
        f.cdn.requests()
    );
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_download_past_the_spool_budget_is_refused_before_fetching() {
    let (mut f, _dir) = spool_fixture(
        "a_download_past_the_spool_budget_is_refused_before_fetching",
        64 * KIB,
    )
    .await;
    let big = served(&f, &plaintext(100_000), "/v/spool/over-budget");
    let (frames, status) = download(&mut f, big).await;
    assert!(frames.is_empty(), "no frame: {frames:?}");
    assert_refused_with(
        status,
        Code::ResourceExhausted,
        "media spool budget exhausted",
    );
    assert!(
        f.cdn.requests().is_empty(),
        "fetched: {:?}",
        f.cdn.requests()
    );

    let plain = plaintext(32_000);
    let small = served(&f, &plain, "/v/spool/within-budget");
    let (frames, status) = download(&mut f, small).await;
    assert_eq!(delivered(&frames, status), plain, "a download that fits");
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_upload_past_the_spool_budget_is_refused_and_uploads_nothing() {
    let (mut f, _dir) = spool_fixture(
        "an_upload_past_the_spool_budget_is_refused_and_uploads_nothing",
        64 * KIB,
    )
    .await;
    let plain = plaintext(200_000);
    let frames = std::iter::once(head(header(&f, pb::MediaType::Image)))
        .chain(plain.chunks(50_000).map(chunk))
        .collect();
    let status = send_media(&mut f, frames)
        .await
        .expect_err("over the budget");
    assert_refused_with(
        Some(status),
        Code::ResourceExhausted,
        "media spool budget exhausted",
    );
    assert!(f.cdn.uploads().is_empty(), "nothing uploaded");

    let frames = vec![
        head(header(&f, pb::MediaType::Image)),
        chunk(&plaintext(16_000)),
    ];
    send_media(&mut f, frames)
        .await
        .expect("a send that fits, after the refusal released its bytes");
    assert_eq!(f.cdn.uploads().len(), 1);
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_held_upload_counts_against_a_download_until_it_ends() {
    let (mut f, _dir) = spool_fixture(
        "a_held_upload_counts_against_a_download_until_it_ends",
        256 * KIB,
    )
    .await;
    let plain = plaintext(100_000);
    let descriptor = served(&f, &plain, "/v/spool/after-the-upload");
    f.cdn.hold_uploads();
    let sends_before = f.mock.client_messages().len();
    let task = spawn_send(&f, 200_000);
    until_the_cdn_holds_an_upload(&f).await;

    // 200 000 bytes in flight leave less than 100 000 of the 256 KiB.
    let (frames, status) = download(&mut f, descriptor.clone()).await;
    assert!(frames.is_empty(), "no frame: {frames:?}");
    assert_refused_with(
        status,
        Code::ResourceExhausted,
        "media spool budget exhausted",
    );

    // The client gives up while its upload is in flight: its bytes go back.
    task.abort();
    let _ = task.await;
    let frames =
        until_it_downloads(&f, &descriptor, "the abandoned upload to release its bytes").await;
    assert_eq!(delivered(&frames, None), plain);
    f.cdn.release_uploads();
    assert_eq!(
        f.mock.client_messages().len(),
        sends_before,
        "the abandoned upload sent no message"
    );
    f.cleanup().await;
}

/// Every name that shows up in `dir` while it runs. A spool file never has
/// one, so the set stays empty for a correct spool.
struct DirWatch {
    stop: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<Vec<String>>,
}

impl DirWatch {
    fn start(dir: PathBuf) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut seen = Vec::new();
            while !flag.load(Ordering::Acquire) {
                for name in entries(&dir) {
                    if !seen.contains(&name) {
                        seen.push(name);
                    }
                }
            }
            seen
        });
        Self { stop, thread }
    }

    fn names_seen(self) -> Vec<String> {
        self.stop.store(true, Ordering::Release);
        self.thread.join().expect("watch thread")
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_transfer_leaves_the_spool_dir_empty_and_the_budget_whole() {
    let (mut f, dir) = spool_fixture(
        "every_transfer_leaves_the_spool_dir_empty_and_the_budget_whole",
        512 * KIB,
    )
    .await;
    let watch = DirWatch::start(dir.path().to_path_buf());

    // A send and a download that succeed.
    let frames = vec![
        head(header(&f, pb::MediaType::Image)),
        chunk(&plaintext(200_000)),
    ];
    send_media(&mut f, frames).await.expect("send");
    let plain = plaintext(200_000);
    let ok = served(&f, &plain, "/v/spool/ok");
    let (frames, status) = download(&mut f, ok).await;
    assert_eq!(delivered(&frames, status), plain);

    // A download the CDN does not have, and one whose MAC fails.
    let missing = pb::MediaDescriptor {
        direct_path: "/v/spool/not-served".into(),
        ..served(&f, &plain, "/v/spool/decoy")
    };
    let (frames, status) = download(&mut f, missing).await;
    assert!(frames.is_empty() && status.is_some(), "a 404 fails");
    let mut enc = wacore::upload::encrypt_media_with_key(&plain, MediaType::Document, Some(&KEY))
        .expect("encrypt");
    let last = enc.data_to_upload.len() - 1;
    enc.data_to_upload[last] ^= 0xff;
    f.cdn.serve("/v/spool/tampered", enc.data_to_upload);
    let tampered = pb::MediaDescriptor {
        direct_path: "/v/spool/tampered".into(),
        ..served(&f, &plain, "/v/spool/decoy")
    };
    let (frames, status) = download(&mut f, tampered).await;
    assert!(frames.is_empty() && status.is_some(), "a bad MAC fails");

    // An upload the client abandons in flight, and one the stream framing ends.
    let probe = served(&f, &plaintext(400_000), "/v/spool/probe");
    f.cdn.hold_uploads();
    let task = spawn_send(&f, 300_000);
    until_the_cdn_holds_an_upload(&f).await;
    let (_, status) = download(&mut f, probe.clone()).await;
    assert_refused_with(
        status,
        Code::ResourceExhausted,
        "media spool budget exhausted",
    );
    task.abort();
    let _ = task.await;
    f.cdn.release_uploads();
    let frames = vec![
        head(header(&f, pb::MediaType::Image)),
        chunk(&plaintext(100_000)),
        head(header(&f, pb::MediaType::Image)),
    ];
    send_media(&mut f, frames)
        .await
        .expect_err("a second header");

    // Whatever was held is back: 400 KB of a 512 KiB budget fits again.
    until_it_downloads(&f, &probe, "every transfer to release its bytes").await;
    let seen = watch.names_seen();
    assert!(seen.is_empty(), "spool files had names: {seen:?}");
    assert!(entries(dir.path()).is_empty());
    f.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_media_type_is_refused_before_the_bytes() {
    let (mut f, _dir) = spool_fixture("a_bad_media_type_is_refused_before_the_bytes", 0).await;
    let over = plaintext(LIMIT as usize + 1);
    let unknown = pb::SendMediaHeader {
        media_type: 42,
        ..header(&f, pb::MediaType::Image)
    };
    let status = send_media(&mut f, vec![head(unknown), chunk(&over)])
        .await
        .expect_err("an unknown type");
    assert_refused_with(
        Some(status),
        Code::InvalidArgument,
        "media_type must be one of MEDIA_TYPE_IMAGE|MEDIA_TYPE_VIDEO|MEDIA_TYPE_AUDIO|\
MEDIA_TYPE_DOCUMENT|MEDIA_TYPE_STICKER, got 42",
    );

    let audio_status = pb::PostStatusMediaChunk {
        part: Some(pb::post_status_media_chunk::Part::Header(
            pb::PostStatusMediaHeader {
                account: f.a(),
                media_type: pb::MediaType::Audio as i32,
                mime_type: "audio/ogg".into(),
                recipients: jids(&[f.peer.pn()]),
                ..Default::default()
            },
        )),
    };
    let bytes = pb::PostStatusMediaChunk {
        part: Some(pb::post_status_media_chunk::Part::Chunk(over)),
    };
    let status = f
        .messages
        .post_status_media(tokio_stream::iter(vec![audio_status, bytes]))
        .await
        .expect_err("an audio status");
    assert_refused_with(
        Some(status),
        Code::InvalidArgument,
        "status media_type must be one of MEDIA_TYPE_IMAGE|MEDIA_TYPE_VIDEO, got MEDIA_TYPE_AUDIO",
    );
    assert!(f.cdn.uploads().is_empty());
    f.cleanup().await;
}
