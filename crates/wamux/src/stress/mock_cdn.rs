//! A media CDN standing in on loopback (#69): a real HTTP/1.1 server on
//! `127.0.0.1:0` that answers a GET for a registered `direct_path` with its
//! bytes (the query string, which carries the auth and token, is ignored) and
//! 404 for anything else. Every request target is recorded, so a test can
//! assert what the client asked for. A POST is an upload (#71): the body is
//! kept, and a later GET of the `direct_path` it was answered with serves it.
//!
//! For the media spool's memory gate (#79) it can also hash an upload without
//! keeping it, and serve a GET from a file, so that a large transfer through
//! it costs the test process no more memory than a buffer.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

/// The read size for a body that is hashed rather than kept (#79).
const HASH_READ: usize = 64 * 1024;

/// What the server shares with its connection tasks. `std` mutexes: every
/// critical section is a map or vec touch with no `.await` inside.
struct CdnState {
    bodies: Mutex<HashMap<String, Served>>,
    requests: Mutex<Vec<String>>,
    uploads: Mutex<Vec<MockUpload>>,
    /// Hash each upload's body and drop it instead of keeping it (#79).
    discard_upload_bodies: AtomicBool,
    /// While true, an upload's answer waits (#79): the client's upload stays
    /// in flight, with every byte already received, until it turns false.
    upload_gate: watch::Sender<bool>,
    /// Uploads received and waiting on `upload_gate`.
    uploads_waiting: AtomicUsize,
}

impl Default for CdnState {
    fn default() -> Self {
        Self {
            bodies: Mutex::default(),
            requests: Mutex::default(),
            uploads: Mutex::default(),
            discard_upload_bodies: AtomicBool::new(false),
            upload_gate: watch::Sender::new(false),
            uploads_waiting: AtomicUsize::new(0),
        }
    }
}

/// What a GET of a registered `direct_path` answers with.
#[derive(Clone)]
enum Served {
    Bytes(Vec<u8>),
    /// Streamed from disk on each GET (#79).
    File(PathBuf),
}

/// A running mock CDN. Drop to stop accepting.
pub struct MockCdn {
    addr: SocketAddr,
    state: Arc<CdnState>,
    accept_task: tokio::task::JoinHandle<()>,
}

impl Drop for MockCdn {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

impl MockCdn {
    /// Bind `127.0.0.1:0` and start accepting.
    pub async fn start() -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(CdnState::default());
        let accept_task = tokio::spawn(accept_loop(listener, state.clone()));
        Ok(Self {
            addr,
            state,
            accept_task,
        })
    }

    /// `127.0.0.1:PORT`: the `hostname` a `<media_conn><host>` answer names.
    pub fn host(&self) -> String {
        self.addr.to_string()
    }

    /// Answer every later GET of `direct_path` with `body`.
    pub fn serve(&self, direct_path: &str, body: Vec<u8>) {
        lock(&self.state.bodies).insert(direct_path.to_string(), Served::Bytes(body));
    }

    /// Answer every later GET of `direct_path` with the bytes of `file`, read
    /// from disk as they are sent (#79): a large body never sits in memory.
    pub fn serve_file(&self, direct_path: &str, file: PathBuf) {
        lock(&self.state.bodies).insert(direct_path.to_string(), Served::File(file));
    }

    /// From now on, hash each upload's body as it arrives and keep only its
    /// length and SHA-256 (#79): `MockUpload::body` is then empty, and the
    /// upload cannot be served back.
    /// From now on, read each upload's body whole and then hold its answer
    /// until `release_uploads` (#79). The client's upload is then in flight
    /// with every byte sent, which is a point a test can wait for:
    /// `uploads_waiting`.
    pub fn hold_uploads(&self) {
        self.state.upload_gate.send_replace(true);
    }

    /// Answer the held uploads, and every later one at once.
    pub fn release_uploads(&self) {
        self.state.upload_gate.send_replace(false);
    }

    /// How many uploads have arrived whole and wait on `hold_uploads`.
    pub fn uploads_waiting(&self) -> usize {
        self.state.uploads_waiting.load(Ordering::SeqCst)
    }

    pub fn discard_upload_bodies(&self) {
        self.state
            .discard_upload_bodies
            .store(true, Ordering::SeqCst);
    }

    /// Every request target received (path plus query), oldest first.
    pub fn requests(&self) -> Vec<String> {
        lock(&self.state.requests).clone()
    }

    /// Every upload (POST) received, oldest first (#71). Each one was answered
    /// 200 with `{"url": .., "direct_path": <its direct_path>}`, the shape the
    /// library's upload reads, and a later GET of that `direct_path` serves
    /// the uploaded bytes back.
    pub fn uploads(&self) -> Vec<MockUpload> {
        lock(&self.state.uploads).clone()
    }
}

/// One upload as the CDN received it.
#[derive(Clone, Debug)]
pub struct MockUpload {
    /// The request path, without the query (`/mms/image/<token>`...).
    pub path: String,
    /// The body: the encrypted media plus its MAC, as the client sent it.
    /// Empty after `discard_upload_bodies`.
    pub body: Vec<u8>,
    /// How many body bytes arrived, kept or not (#79).
    pub body_len: u64,
    /// The SHA-256 of the body, kept or not (#79).
    pub body_sha256: [u8; 32],
    /// The `direct_path` the CDN answered with; unique per upload.
    pub direct_path: String,
}

/// Lock, recovering from poison: a panicked connection task must not hide the
/// requests a test wants to assert on.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

async fn accept_loop(listener: TcpListener, state: Arc<CdnState>) {
    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(answer_one_request(stream, state.clone()));
    }
}

/// One request as read off the wire: the request line, and the body a POST
/// declared with `Content-Length`, kept or only hashed.
struct CdnRequest {
    method: String,
    target: String,
    body: Vec<u8>,
    body_len: u64,
    body_sha256: [u8; 32],
}

/// Read one request, answer it, close. `Connection: close` gives ureq a body
/// end that does not depend on keep-alive bookkeeping.
async fn answer_one_request(stream: TcpStream, state: Arc<CdnState>) {
    let mut reader = BufReader::new(stream);
    let discard = state.discard_upload_bodies.load(Ordering::SeqCst);
    let Some(request) = read_request(&mut reader, discard).await else {
        return;
    };
    let path = request
        .target
        .split('?')
        .next()
        .unwrap_or_default()
        .to_string();
    lock(&state.requests).push(request.target.clone());
    if request.method == "POST" && !request.target.contains("resume=1") {
        wait_while_uploads_are_held(&state).await;
    }
    let answer = match request.method.as_str() {
        "GET" => lock(&state.bodies).get(&path).cloned(),
        "POST" => Some(Served::Bytes(answer_upload(&state, &path, &request))),
        _ => None,
    };
    let mut stream = reader.into_inner();
    let _ = write_response(&mut stream, answer).await;
    let _ = stream.shutdown().await;
}

/// Park an upload's answer while `hold_uploads` is on, counted in
/// `uploads_waiting` (#79).
async fn wait_while_uploads_are_held(state: &CdnState) {
    let mut gate = state.upload_gate.subscribe();
    if !*gate.borrow() {
        return;
    }
    state.uploads_waiting.fetch_add(1, Ordering::SeqCst);
    let _ = gate.wait_for(|held| !*held).await;
    state.uploads_waiting.fetch_sub(1, Ordering::SeqCst);
}

/// What a POST gets back. The library's resume check (`&resume=1`, a POST with
/// no body, upload.rs:84) is answered `{}`: nothing uploaded yet. Anything
/// else is the upload itself: keep it, and name the `direct_path` it is served
/// at, unique per upload so two uploads never read each other's bytes.
fn answer_upload(state: &CdnState, path: &str, request: &CdnRequest) -> Vec<u8> {
    if request.target.contains("resume=1") {
        return b"{}".to_vec();
    }
    let mut uploads = lock(&state.uploads);
    let direct_path = format!("/v/mock/upload-{}", uploads.len() + 1);
    lock(&state.bodies).insert(direct_path.clone(), Served::Bytes(request.body.clone()));
    uploads.push(MockUpload {
        path: path.to_string(),
        body: request.body.clone(),
        body_len: request.body_len,
        body_sha256: request.body_sha256,
        direct_path: direct_path.clone(),
    });
    // RawUploadResponse (upload.rs:363) reads exactly these two fields.
    format!(r#"{{"url":"https://mock-cdn.invalid{direct_path}","direct_path":"{direct_path}"}}"#)
        .into_bytes()
}

/// The request line, the headers and, for a body-carrying request, the body
/// (or, with `discard`, only its length and hash). `None` for anything that is
/// not a well-formed request.
async fn read_request(reader: &mut BufReader<TcpStream>, discard: bool) -> Option<CdnRequest> {
    let mut line = String::new();
    reader.read_line(&mut line).await.ok()?;
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next()?.to_string(), parts.next()?.to_string());
    let length = read_headers(reader).await?;
    let (body, body_sha256) = read_body(reader, length, discard).await?;
    Some(CdnRequest {
        method,
        target,
        body,
        body_len: length as u64,
        body_sha256,
    })
}

/// `length` body bytes, hashed as they arrive; kept unless `discard`.
async fn read_body(
    reader: &mut BufReader<TcpStream>,
    length: usize,
    discard: bool,
) -> Option<(Vec<u8>, [u8; 32])> {
    let mut hasher = Sha256::new();
    let mut kept = Vec::with_capacity(if discard { 0 } else { length });
    let mut buffer = vec![0u8; HASH_READ.min(length.max(1))];
    let mut left = length;
    while left > 0 {
        let take = left.min(buffer.len());
        reader.read_exact(&mut buffer[..take]).await.ok()?;
        hasher.update(&buffer[..take]);
        if !discard {
            kept.extend_from_slice(&buffer[..take]);
        }
        left -= take;
    }
    let digest = hasher.finalize();
    let mut sha256 = [0u8; 32];
    sha256.copy_from_slice(&digest);
    Some((kept, sha256))
}

/// Drain the headers; the `Content-Length` they declare, 0 when absent. The
/// library's uploads always set it (ureq-client execute_upload, lib.rs:344),
/// so a chunked body never reaches here.
async fn read_headers(reader: &mut BufReader<TcpStream>) -> Option<usize> {
    let mut length = 0;
    loop {
        let mut header = String::new();
        let read = reader.read_line(&mut header).await.ok()?;
        if read == 0 || header == "\r\n" || header == "\n" {
            return Some(length);
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().ok()?;
        }
    }
}

/// Write the answer: 200 with the body, or 404 with none. A file body is
/// copied from disk after the head, never read whole (#79).
async fn write_response(stream: &mut TcpStream, answer: Option<Served>) -> std::io::Result<()> {
    match answer {
        None => stream.write_all(&response_head("404 Not Found", 0)).await,
        Some(Served::Bytes(bytes)) => {
            let head = response_head("200 OK", bytes.len() as u64);
            stream.write_all(&[head, bytes].concat()).await
        }
        Some(Served::File(path)) => {
            let mut file = tokio::fs::File::open(&path).await?;
            let length = file.metadata().await?.len();
            stream.write_all(&response_head("200 OK", length)).await?;
            tokio::io::copy(&mut file, stream).await.map(|_| ())
        }
    }
}

fn response_head(status: &str, length: u64) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Length: {length}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n"
    )
    .into_bytes()
}
