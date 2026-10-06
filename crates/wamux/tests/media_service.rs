//! MediaService through the socket (#69): `DownloadMedia` over a real Unix
//! socket and tonic client, with the account logged in against `MockWaServer`
//! and the media served by `MockCdn`, a real HTTP server on loopback, through
//! the production HTTP client (`LoopbackHttpClient` sends loopback `https://`
//! as `http://`; the URL the library builds is always https).
//!
//! The media is encrypted at test time with the library's own encryptor and a
//! fixed key, so every assertion is against a plaintext the test chose. These
//! tests go through the socket only, so they hold across the media spool
//! (#79). Honors `WAMUX_TEST_ENGINE`; `scripts/ci.sh` runs it on both engines.
#![cfg(feature = "stress")]

use tonic::Code;
use tonic::transport::Channel;
use wacore::download::MediaType;
use wamux::proto::v1 as pb;
use wamux::proto::v1::media_service_client::MediaServiceClient;
use wamux::stress::{MockCdn, MockWaServer};

// Only a subset of the shared helpers is used per test binary.
#[allow(dead_code)]
mod common;

const KEY: [u8; 32] = [0x5a; 32];
const CHUNK: usize = 64 * 1024;

struct Fixture {
    mock: MockWaServer,
    cdn: MockCdn,
    logged: common::LoggedIn,
    media: MediaServiceClient<Channel>,
    account: pb::AccountRef,
}

async fn fixture(test: &str) -> Fixture {
    let mock = MockWaServer::start().await.expect("start mock");
    let cdn = MockCdn::start().await.expect("start cdn");
    let prefix = common::test_prefix("media_service", test);
    let logged = common::logged_in_client(&mock, &prefix).await;
    mock.answer_iq(
        "w:m",
        "set",
        "media_conn",
        common::mock_wire::media_conn(&cdn.host()),
    );
    let channel = common::serve_registry(logged.registry.clone()).await;
    let account = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::Uuid(logged.handle.uuid.to_string())),
    };
    Fixture {
        mock,
        cdn,
        logged,
        media: MediaServiceClient::new(channel),
        account,
    }
}

/// A deterministic plaintext of `len` bytes.
fn plaintext(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 % 251) as u8).collect()
}

/// Encrypt `plain` as `media_type`, serve the ciphertext at `path`, and return
/// the descriptor an inbound message event would carry for it: `wire` is the
/// descriptor's `media_type`, and its name labels the mime type.
fn encrypted(
    f: &Fixture,
    plain: &[u8],
    wire: pb::MediaType,
    media_type: MediaType,
    path: &str,
) -> pb::MediaDescriptor {
    let enc =
        wacore::upload::encrypt_media_with_key(plain, media_type, Some(&KEY)).expect("encrypt");
    f.cdn.serve(path, enc.data_to_upload);
    pb::MediaDescriptor {
        direct_path: path.into(),
        media_key: enc.media_key.to_vec(),
        file_enc_sha256: enc.file_enc_sha256.to_vec(),
        file_sha256: enc.file_sha256.to_vec(),
        file_length: plain.len() as u64,
        mime_type: format!("test/{}", wire.as_str_name()),
        media_type: wire as i32,
    }
}

/// The SHA-256 of `plain`, as the encryptor computes `file_sha256`.
fn sha256_of(plain: &[u8]) -> Vec<u8> {
    wacore::upload::encrypt_media_with_key(plain, MediaType::Image, Some(&KEY))
        .expect("hash through the encryptor")
        .file_sha256
        .to_vec()
}

/// Every frame the stream delivered, and the status that ended it if it was
/// not OK. Kept apart so a test can assert that a failure delivered nothing.
async fn download(
    f: &mut Fixture,
    descriptor: Option<pb::MediaDescriptor>,
) -> (Vec<pb::MediaChunk>, Option<tonic::Status>) {
    download_as(f, f.account.clone(), descriptor).await
}

async fn download_as(
    f: &mut Fixture,
    account: pb::AccountRef,
    descriptor: Option<pb::MediaDescriptor>,
) -> (Vec<pb::MediaChunk>, Option<tonic::Status>) {
    let request = pb::DownloadMediaRequest {
        account: Some(account),
        descriptor,
    };
    let mut stream = match f.media.download_media(request).await {
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

fn meta_of(frame: &pb::MediaChunk) -> &pb::MediaMeta {
    match &frame.part {
        Some(pb::media_chunk::Part::Meta(meta)) => meta,
        other => panic!("the first frame must be the meta, got {other:?}"),
    }
}

/// The chunk frames after the meta, as byte vectors.
fn chunks_of(frames: &[pb::MediaChunk]) -> Vec<Vec<u8>> {
    frames[1..]
        .iter()
        .map(|frame| match &frame.part {
            Some(pb::media_chunk::Part::Chunk(bytes)) => bytes.clone(),
            other => panic!("every frame after the meta is a chunk, got {other:?}"),
        })
        .collect()
}

/// A successful download: meta first (mime and length), then the plaintext.
fn assert_delivered(
    frames: &[pb::MediaChunk],
    status: Option<tonic::Status>,
    mime: &str,
    plain: &[u8],
) {
    assert!(status.is_none(), "download failed: {status:?}");
    let meta = meta_of(&frames[0]);
    assert_eq!(meta.mime_type, mime);
    assert_eq!(meta.file_length, plain.len() as u64);
    assert_eq!(chunks_of(frames).concat(), plain);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_streams_meta_then_64k_chunks_of_the_decrypted_bytes() {
    let mut f = fixture("download_streams_meta_then_64k_chunks").await;
    let plain = plaintext(150_000);
    let path = "/v/t62.7118-24/wamux-69/image";
    let descriptor = encrypted(&f, &plain, pb::MediaType::Image, MediaType::Image, path);
    let (frames, status) = download(&mut f, Some(descriptor)).await;
    assert_delivered(&frames, status, "test/MEDIA_TYPE_IMAGE", &plain);
    let sizes: Vec<usize> = chunks_of(&frames).iter().map(Vec::len).collect();
    assert_eq!(sizes, [CHUNK, CHUNK, 150_000 - 2 * CHUNK]);
    let asked = f.cdn.requests();
    assert!(
        asked.iter().any(|r| r.starts_with(&format!("{path}?"))
            && r.contains("auth=mock-auth")
            && r.contains("token=")),
        "the CDN must see the direct path with the media auth and token: {asked:?}"
    );
    let media_conn_iqs = f.mock.client_iqs().into_iter().filter(|iq| {
        iq.attrs
            .get("xmlns")
            .map(|v| v.as_str().into_owned())
            .as_deref()
            == Some("w:m")
    });
    assert!(
        media_conn_iqs.count() >= 1,
        "the route came from a media_conn IQ"
    );
    f.logged.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_decrypts_every_media_type() {
    let mut f = fixture("download_decrypts_every_media_type").await;
    let kinds = [
        (pb::MediaType::Image, MediaType::Image),
        (pb::MediaType::Video, MediaType::Video),
        (pb::MediaType::Audio, MediaType::Audio),
        (pb::MediaType::Document, MediaType::Document),
        (pb::MediaType::Sticker, MediaType::Sticker),
    ];
    for (i, (wire, media_type)) in kinds.into_iter().enumerate() {
        let plain = plaintext(10_000 + i);
        let kind = wire.as_str_name();
        let descriptor = encrypted(&f, &plain, wire, media_type, &format!("/v/wamux-69/{kind}"));
        let (frames, status) = download(&mut f, Some(descriptor)).await;
        assert_delivered(&frames, status, &format!("test/{kind}"), &plain);
    }
    f.logged.cleanup().await;
}

/// #58: a received sticker pack's ZIP and its thumbnail download with their
/// own media types (each derives a different key from the same media key).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_serves_both_sticker_pack_types() {
    let mut f = fixture("download_serves_both_sticker_pack_types").await;
    let kinds = [
        (pb::MediaType::StickerPack, MediaType::StickerPack),
        (
            pb::MediaType::StickerPackThumbnail,
            MediaType::StickerPackThumbnail,
        ),
    ];
    for (wire, media_type) in kinds {
        let plain = plaintext(70_000);
        let kind = wire.as_str_name();
        let descriptor = encrypted(&f, &plain, wire, media_type, &format!("/v/wamux-69/{kind}"));
        let (frames, status) = download(&mut f, Some(descriptor)).await;
        assert_delivered(&frames, status, &format!("test/{kind}"), &plain);
    }
    f.logged.cleanup().await;
}

/// #6: channel media carries no key. It is served in the clear and the
/// library verifies it against `file_sha256` instead of skipping the check.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_of_keyless_channel_media_checks_file_sha256() {
    let mut f = fixture("download_of_keyless_channel_media").await;
    let plain = plaintext(20_000);
    let path = "/v/wamux-69/channel-image";
    f.cdn.serve(path, plain.clone());
    let descriptor = pb::MediaDescriptor {
        direct_path: path.into(),
        file_sha256: sha256_of(&plain),
        file_length: plain.len() as u64,
        mime_type: "image/jpeg".into(),
        media_type: pb::MediaType::Image as i32,
        ..Default::default()
    };
    let (frames, status) = download(&mut f, Some(descriptor)).await;
    assert_delivered(&frames, status, "image/jpeg", &plain);
    f.logged.cleanup().await;
}

/// The RPC decrypts before the first frame, so a ciphertext whose MAC fails
/// ends the call with an error and nothing that could pass for media. The code
/// is Unavailable today (a library error with no server code).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tampered_ciphertext_fails_without_a_single_frame() {
    let mut f = fixture("a_tampered_ciphertext_fails").await;
    let plain = plaintext(80_000);
    let path = "/v/wamux-69/tampered";
    let enc = wacore::upload::encrypt_media_with_key(&plain, MediaType::Image, Some(&KEY))
        .expect("encrypt");
    let mut tampered = enc.data_to_upload.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0x01;
    f.cdn.serve(path, tampered);
    let descriptor = pb::MediaDescriptor {
        direct_path: path.into(),
        media_key: enc.media_key.to_vec(),
        file_enc_sha256: enc.file_enc_sha256.to_vec(),
        file_sha256: enc.file_sha256.to_vec(),
        file_length: plain.len() as u64,
        mime_type: "image/jpeg".into(),
        media_type: pb::MediaType::Image as i32,
    };
    let (frames, status) = download(&mut f, Some(descriptor)).await;
    assert!(
        frames.is_empty(),
        "no frame may reach the client: {} did",
        frames.len()
    );
    assert_eq!(
        status.expect("the call must fail").code(),
        Code::Unavailable
    );
    f.logged.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_keyless_download_with_a_wrong_hash_fails() {
    let mut f = fixture("a_keyless_download_with_a_wrong_hash").await;
    let path = "/v/wamux-69/channel-wrong-hash";
    f.cdn.serve(path, plaintext(5_000));
    let descriptor = pb::MediaDescriptor {
        direct_path: path.into(),
        file_sha256: sha256_of(&plaintext(5_001)),
        file_length: 5_000,
        mime_type: "image/jpeg".into(),
        media_type: pb::MediaType::Image as i32,
        ..Default::default()
    };
    let (frames, status) = download(&mut f, Some(descriptor)).await;
    assert!(
        frames.is_empty(),
        "unverified bytes may not reach the client"
    );
    assert_eq!(
        status.expect("the call must fail").code(),
        Code::Unavailable
    );
    f.logged.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cdn_404_fails_with_unavailable() {
    let mut f = fixture("a_cdn_404_fails_with_unavailable").await;
    let path = "/v/wamux-69/never-served";
    let descriptor = pb::MediaDescriptor {
        direct_path: path.into(),
        media_key: KEY.to_vec(),
        file_enc_sha256: vec![1; 32],
        file_sha256: vec![2; 32],
        file_length: 10,
        mime_type: "image/jpeg".into(),
        media_type: pb::MediaType::Image as i32,
    };
    let (frames, status) = download(&mut f, Some(descriptor)).await;
    assert!(frames.is_empty());
    assert_eq!(
        status.expect("the call must fail").code(),
        Code::Unavailable
    );
    assert!(
        f.cdn.requests().iter().any(|r| r.starts_with(path)),
        "the CDN was asked"
    );
    f.logged.cleanup().await;
}

/// A descriptor that is valid in shape, for the account and argument tests.
fn some_descriptor() -> pb::MediaDescriptor {
    pb::MediaDescriptor {
        direct_path: "/v/wamux-69/anything".into(),
        media_key: KEY.to_vec(),
        file_enc_sha256: vec![1; 32],
        file_sha256: vec![2; 32],
        file_length: 10,
        mime_type: "image/jpeg".into(),
        media_type: pb::MediaType::Image as i32,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_account_is_not_found() {
    let mut f = fixture("unknown_account_is_not_found").await;
    let unknown = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::Uuid(uuid::Uuid::new_v4().to_string())),
    };
    let (frames, status) = download_as(&mut f, unknown, Some(some_descriptor())).await;
    assert!(frames.is_empty());
    assert_eq!(status.expect("must fail").code(), Code::NotFound);
    assert!(f.cdn.requests().is_empty(), "nothing goes to the CDN");
    f.logged.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_account_not_connected_is_failed_precondition() {
    let mut f = fixture("an_account_not_connected").await;
    let prefix = common::test_prefix("media_service", "an_account_not_connected_idle");
    common::sweep_orphans(f.logged.registry.storage(), &prefix).await;
    let idle = f
        .logged
        .registry
        .create_account(Some(&format!("{prefix}idle")))
        .await
        .expect("idle account");
    let idle_ref = pb::AccountRef {
        r#ref: Some(pb::account_ref::Ref::Uuid(idle.uuid.to_string())),
    };
    let (frames, status) = download_as(&mut f, idle_ref, Some(some_descriptor())).await;
    assert!(frames.is_empty());
    assert_eq!(status.expect("must fail").code(), Code::FailedPrecondition);
    assert!(f.cdn.requests().is_empty(), "nothing goes to the CDN");
    f.logged
        .registry
        .delete(&idle)
        .await
        .expect("delete the idle account");
    f.logged.cleanup().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_descriptor_is_invalid_argument() {
    let mut f = fixture("a_missing_descriptor_is_invalid_argument").await;
    let (frames, status) = download(&mut f, None).await;
    assert!(frames.is_empty());
    assert_eq!(status.expect("must fail").code(), Code::InvalidArgument);
    assert!(f.cdn.requests().is_empty(), "nothing goes to the CDN");
    f.logged.cleanup().await;
}

// #127: unset, unknown or a number outside the enum is refused before the
// CDN, with the one message shape naming the value.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_media_type_is_invalid_argument() {
    let mut f = fixture("an_unknown_media_type_is_invalid_argument").await;
    let refused = [
        (0, "MEDIA_TYPE_UNSPECIFIED"),
        (1, "MEDIA_TYPE_UNKNOWN"),
        (42, "42"),
    ];
    for (value, shown) in refused {
        let descriptor = pb::MediaDescriptor {
            media_type: value,
            ..some_descriptor()
        };
        let (frames, status) = download(&mut f, Some(descriptor)).await;
        assert!(frames.is_empty(), "{shown}");
        let status = status.expect("must fail");
        assert_eq!(status.code(), Code::InvalidArgument, "{shown}");
        assert_eq!(
            status.message(),
            format!(
                "media_type must be one of MEDIA_TYPE_IMAGE|MEDIA_TYPE_VIDEO|MEDIA_TYPE_AUDIO|\
MEDIA_TYPE_DOCUMENT|MEDIA_TYPE_STICKER|MEDIA_TYPE_STICKER_PACK|\
MEDIA_TYPE_STICKER_PACK_THUMBNAIL, got {shown}"
            )
        );
    }
    assert!(f.cdn.requests().is_empty(), "nothing goes to the CDN");
    f.logged.cleanup().await;
}
