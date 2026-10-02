//! A media CDN standing in on loopback (#69): a real HTTP/1.1 server on
//! `127.0.0.1:0` that answers a GET for a registered `direct_path` with its
//! bytes (the query string, which carries the auth and token, is ignored) and
//! 404 for anything else. Every request target is recorded, so a test can
//! assert what the client asked for. A POST is an upload (#71): the body is
//! kept, and a later GET of the `direct_path` it was answered with serves it.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// What the server shares with its connection tasks. `std` mutexes: every
/// critical section is a map or vec touch with no `.await` inside.
#[derive(Default)]
struct CdnState {
    bodies: Mutex<HashMap<String, Vec<u8>>>,
    requests: Mutex<Vec<String>>,
    uploads: Mutex<Vec<MockUpload>>,
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
        lock(&self.state.bodies).insert(direct_path.to_string(), body);
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
    pub body: Vec<u8>,
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
/// declared with `Content-Length`.
struct CdnRequest {
    method: String,
    target: String,
    body: Vec<u8>,
}

/// Read one request, answer it, close. `Connection: close` gives ureq a body
/// end that does not depend on keep-alive bookkeeping.
async fn answer_one_request(stream: TcpStream, state: Arc<CdnState>) {
    let mut reader = BufReader::new(stream);
    let Some(request) = read_request(&mut reader).await else {
        return;
    };
    let path = request
        .target
        .split('?')
        .next()
        .unwrap_or_default()
        .to_string();
    lock(&state.requests).push(request.target.clone());
    let answer = match request.method.as_str() {
        "GET" => lock(&state.bodies).get(&path).cloned(),
        "POST" => Some(answer_upload(&state, &path, &request)),
        _ => None,
    };
    let mut stream = reader.into_inner();
    let _ = stream.write_all(&render_response(answer)).await;
    let _ = stream.shutdown().await;
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
    lock(&state.bodies).insert(direct_path.clone(), request.body.clone());
    uploads.push(MockUpload {
        path: path.to_string(),
        body: request.body.clone(),
        direct_path: direct_path.clone(),
    });
    // RawUploadResponse (upload.rs:363) reads exactly these two fields.
    format!(r#"{{"url":"https://mock-cdn.invalid{direct_path}","direct_path":"{direct_path}"}}"#)
        .into_bytes()
}

/// The request line, the headers and, for a body-carrying request, the body.
/// `None` for anything that is not a well-formed request.
async fn read_request(reader: &mut BufReader<TcpStream>) -> Option<CdnRequest> {
    let mut line = String::new();
    reader.read_line(&mut line).await.ok()?;
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next()?.to_string(), parts.next()?.to_string());
    let length = read_headers(reader).await?;
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).await.ok()?;
    Some(CdnRequest {
        method,
        target,
        body,
    })
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

fn render_response(body: Option<Vec<u8>>) -> Vec<u8> {
    let (status, body) = match body {
        Some(bytes) => ("200 OK", bytes),
        None => ("404 Not Found", Vec::new()),
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
        body.len()
    );
    [head.into_bytes(), body].concat()
}
