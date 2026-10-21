//! `fetch_url_capped` (#209): the GET the media probes run, against a one-shot
//! HTTP server on loopback. The caps the binaries pass (16 and 32 MiB) are over
//! the 10 MiB body limit ureq 3 applies by default, so a body between the two
//! must still come back whole.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use wamux_tools::media_kit::{MediaKitError, fetch_url_capped};

const MIB: usize = 1024 * 1024;

/// Answer one request with `status` and `body`, then close. Returns the URL.
fn serve_once(status: &'static str, body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let url = format!(
        "http://{}/media",
        listener.local_addr().expect("local addr")
    );
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        // The request head ends at the first empty line.
        while reader.read_line(&mut line).expect("read request") > 2 {
            line.clear();
        }
        let mut stream = reader.into_inner();
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        // The client may hang up early on a body over its cap.
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(&body);
    });
    url
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

#[tokio::test]
async fn fetch_url_capped_returns_a_body_of_exactly_the_cap() {
    let body = pattern(4096);
    let url = serve_once("200 OK", body.clone());
    let fetched = fetch_url_capped(&url, 4096).await.expect("fetch");
    assert_eq!(fetched, body);
}

#[tokio::test]
async fn fetch_url_capped_refuses_a_body_one_byte_over_the_cap() {
    let url = serve_once("200 OK", pattern(4097));
    let refused = fetch_url_capped(&url, 4096).await;
    assert!(
        matches!(refused, Err(MediaKitError::TooLarge { cap: 4096, .. })),
        "{refused:?}"
    );
}

#[tokio::test]
async fn fetch_url_capped_reads_a_body_past_ten_mib_under_a_larger_cap() {
    let body = pattern(11 * MIB);
    let url = serve_once("200 OK", body.clone());
    let fetched = fetch_url_capped(&url, 16 * MIB)
        .await
        .expect("fetch 11 MiB");
    assert_eq!(fetched.len(), body.len());
    assert!(fetched == body, "the 11 MiB body came back altered");
}

#[tokio::test]
async fn fetch_url_capped_reports_an_error_status_as_a_fetch_failure() {
    let url = serve_once("404 Not Found", b"gone".to_vec());
    let failed = fetch_url_capped(&url, 4096).await;
    assert!(
        matches!(&failed, Err(MediaKitError::Fetch { url: u, .. }) if *u == url),
        "{failed:?}"
    );
}
