//! A media CDN standing in on loopback (#69): a real HTTP/1.1 server on
//! `127.0.0.1:0` that answers a GET for a registered `direct_path` with its
//! bytes (the query string, which carries the auth and token, is ignored) and
//! 404 for anything else. Every request target is recorded, so a test can
//! assert what the client asked for.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// What the server shares with its connection tasks. `std` mutexes: every
/// critical section is a map or vec touch with no `.await` inside.
#[derive(Default)]
struct CdnState {
    bodies: Mutex<HashMap<String, Vec<u8>>>,
    requests: Mutex<Vec<String>>,
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

/// Read one request, answer it, close. `Connection: close` gives ureq a body
/// end that does not depend on keep-alive bookkeeping.
async fn answer_one_request(stream: TcpStream, state: Arc<CdnState>) {
    let mut reader = BufReader::new(stream);
    let Some(target) = read_request_target(&mut reader).await else {
        return;
    };
    let path = target.split('?').next().unwrap_or_default().to_string();
    lock(&state.requests).push(target);
    let body = lock(&state.bodies).get(&path).cloned();
    let mut stream = reader.into_inner();
    let _ = stream.write_all(&render_response(body)).await;
    let _ = stream.shutdown().await;
}

/// The request-target of a GET, after draining the headers. `None` for
/// anything that is not a well-formed GET request line.
async fn read_request_target(reader: &mut BufReader<TcpStream>) -> Option<String> {
    let mut line = String::new();
    reader.read_line(&mut line).await.ok()?;
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next()?, parts.next()?.to_string());
    drain_headers(reader).await?;
    (method == "GET").then_some(target)
}

async fn drain_headers(reader: &mut BufReader<TcpStream>) -> Option<()> {
    loop {
        let mut header = String::new();
        let read = reader.read_line(&mut header).await.ok()?;
        if read == 0 || header == "\r\n" || header == "\n" {
            return Some(());
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
