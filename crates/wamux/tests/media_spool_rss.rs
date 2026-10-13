//! The media spool's memory gate (#79): sending and downloading a file many
//! times larger than a fixed bound must not grow the process's peak RSS by
//! that bound. Before the spool, SendMedia held the plaintext and the
//! ciphertext of the whole file at once, and DownloadMedia the whole
//! plaintext, so 64 MiB grew the peak by well over 100 MiB.
//!
//! A test binary of its own: the peak (`VmHWM`) belongs to the process, and
//! no other test may allocate while it is measured. The mock CDN hashes the
//! upload instead of keeping it and serves the download from a file, and the
//! client generates and hashes its bytes as they stream, so nothing in the
//! test itself holds the file. Fixtures go under `CARGO_TARGET_TMPDIR` (a real
//! disk), never `/tmp`, which may be RAM.
#![cfg(feature = "stress")]

#[allow(dead_code)]
#[path = "common/mod.rs"]
mod common;
#[allow(dead_code)]
#[path = "messaging_service/harness.rs"]
mod harness;

use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use wacore::download::MediaType;
use wacore::upload::{EncryptedMediaInfo, MediaEncryptor, encrypted_len};
use wamux::config::Config;
use wamux::proto::v1 as pb;

const MIB: usize = 1024 * 1024;
/// The file sent and the file downloaded.
const SIZE: usize = 64 * MIB;
const CHUNK: usize = 64 * 1024;
/// The most the peak may grow over the whole send + download, in KiB: an
/// absolute number, a fraction of `SIZE`.
const GROWTH_BOUND_KIB: u64 = 24 * 1024;
const KEY: [u8; 32] = [0x64; 32];
const DOWNLOAD_PATH: &str = "/v/spool-rss/download";

/// Chunk `index` of the deterministic plaintext.
fn plain_chunk(index: usize) -> Vec<u8> {
    (0..CHUNK)
        .map(|j| ((index * 7 + j * 31) % 251) as u8)
        .collect()
}

fn status_kib(field: &str) -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").expect("/proc/self/status");
    let line = status
        .lines()
        .find_map(|line| line.strip_prefix(field))
        .unwrap_or_else(|| panic!("no {field} line"));
    line.trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .expect("a kB count")
}

/// Reset the peak to the current RSS (Linux 4.0+), and return it.
fn reset_peak_rss_kib() -> u64 {
    std::fs::write("/proc/self/clear_refs", "5").expect("reset VmHWM");
    status_kib("VmHWM:")
}

/// Encrypt `SIZE` bytes of the plaintext to `path`, a chunk at a time.
fn encrypt_to_file(path: &Path) -> EncryptedMediaInfo {
    let file = std::fs::File::create(path).expect("ciphertext file");
    let mut out = std::io::BufWriter::new(file);
    let mut encryptor = MediaEncryptor::with_key(KEY, MediaType::Document).expect("encryptor");
    for index in 0..SIZE / CHUNK {
        encryptor
            .update_to_writer(&plain_chunk(index), &mut out)
            .expect("encrypt a chunk");
    }
    let info = encryptor.finalize_to_writer(&mut out).expect("finalize");
    out.flush().expect("flush");
    info
}

fn send_frames(f: &harness::Fixture) -> impl Iterator<Item = pb::SendMediaChunk> + Send + 'static {
    let header = pb::SendMediaHeader {
        account: f.a(),
        to: harness::jid(f.peer.pn()),
        mime_type: "application/octet-stream".into(),
        filename: "big.bin".into(),
        media_type: pb::MediaType::Document as i32,
        ..Default::default()
    };
    let head = pb::SendMediaChunk {
        part: Some(pb::send_media_chunk::Part::Header(header)),
    };
    let chunks = (0..SIZE / CHUNK).map(|index| pb::SendMediaChunk {
        part: Some(pb::send_media_chunk::Part::Chunk(plain_chunk(index))),
    });
    std::iter::once(head).chain(chunks)
}

/// Download `descriptor`, hashing as it arrives: (bytes, SHA-256).
async fn download_hashed(
    f: &mut harness::Fixture,
    descriptor: pb::MediaDescriptor,
) -> (u64, [u8; 32]) {
    let request = pb::DownloadMediaRequest {
        account: f.a(),
        descriptor: Some(descriptor),
    };
    let mut stream = f
        .media
        .download_media(request)
        .await
        .expect("download")
        .into_inner();
    let (mut total, mut hasher) = (0u64, Sha256::new());
    while let Some(frame) = stream.message().await.expect("a frame") {
        if let Some(pb::media_chunk::Part::Chunk(bytes)) = frame.part {
            total += bytes.len() as u64;
            hasher.update(&bytes);
        }
    }
    let mut sha256 = [0u8; 32];
    sha256.copy_from_slice(&hasher.finalize());
    (total, sha256)
}

fn spool_config(work: &Path) -> (Config, PathBuf) {
    let spool = work.join("spool");
    std::fs::create_dir(&spool).expect("spool dir");
    let config = Config {
        media_max_bytes: 2 * SIZE as u64,
        media_spool_dir: spool.to_str().expect("utf-8").to_string(),
        media_spool_max_bytes: 4 * SIZE as u64,
        ..Config::default()
    };
    (config, spool)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sending_and_downloading_64_mib_grows_peak_rss_by_less_than_24_mib() {
    let work = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("work dir on disk");
    let (config, _spool) = spool_config(work.path());
    let mut f = harness::fixture_with(
        "sending_and_downloading_64_mib_grows_peak_rss_by_less_than_24_mib",
        config,
    )
    .await;
    f.cdn.discard_upload_bodies();
    let ciphertext = work.path().join("download.enc");
    let info = encrypt_to_file(&ciphertext);
    f.cdn.serve_file(DOWNLOAD_PATH, ciphertext);
    let descriptor = pb::MediaDescriptor {
        direct_path: DOWNLOAD_PATH.into(),
        media_key: KEY.to_vec(),
        file_enc_sha256: info.file_enc_sha256.to_vec(),
        file_sha256: info.file_sha256.to_vec(),
        file_length: SIZE as u64,
        mime_type: "application/octet-stream".into(),
        media_type: pb::MediaType::Document as i32,
    };

    let baseline = reset_peak_rss_kib();
    let frames = send_frames(&f);
    f.messages
        .send_media(tokio_stream::iter(frames))
        .await
        .expect("send 64 MiB");
    let uploads = f.cdn.uploads();
    assert_eq!(uploads.len(), 1, "one upload");
    assert_eq!(
        uploads[0].body_len,
        encrypted_len(SIZE) as u64,
        "the whole file went up"
    );
    let after_send = status_kib("VmHWM:");
    let (total, sha256) = download_hashed(&mut f, descriptor).await;
    let peak = status_kib("VmHWM:");

    assert_eq!(total, SIZE as u64, "the whole file came back");
    assert_eq!(sha256, info.file_sha256, "and it is the file");
    let (send_growth, growth) = (
        after_send.saturating_sub(baseline),
        peak.saturating_sub(baseline),
    );
    assert!(
        growth < GROWTH_BOUND_KIB,
        "peak RSS grew {growth} KiB ({send_growth} KiB by the send) moving {} MiB; the bound is {GROWTH_BOUND_KIB} KiB",
        SIZE / MIB
    );
    f.cleanup().await;
}
