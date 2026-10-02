//! The rewrite is the only thing standing between a stress build and a
//! downgraded request, so its edges are pinned here (#69).

use super::loopback_to_plain_http;

#[test]
fn loopback_https_becomes_plain_http() {
    let cases = [
        (
            "https://127.0.0.1:41234/v/t62/enc?auth=a&token=t",
            "http://127.0.0.1:41234/v/t62/enc?auth=a&token=t",
        ),
        ("https://127.0.0.1/x", "http://127.0.0.1/x"),
        ("https://localhost:8080/x", "http://localhost:8080/x"),
        ("https://[::1]:9/x", "http://[::1]:9/x"),
    ];
    for (url, expected) in cases {
        assert_eq!(
            loopback_to_plain_http(url).as_deref(),
            Some(expected),
            "{url}"
        );
    }
}

#[test]
fn a_real_or_lookalike_host_is_left_alone() {
    let untouched = [
        "https://mmg.whatsapp.net/v/t62.7118-24/enc?auth=a&token=t",
        "https://127.0.0.1.evil.example/x",
        "https://localhost.evil.example:443/x",
        "https://evil.example/127.0.0.1/x",
        "https://user@127.0.0.1.evil.example/x",
        "http://127.0.0.1:41234/x",
        "https://",
        "",
    ];
    for url in untouched {
        assert_eq!(loopback_to_plain_http(url), None, "{url:?}");
    }
}
