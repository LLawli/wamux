//! The rewrite is the only thing standing between a stress build and a
//! downgraded request, so its edges are pinned here (#69). The refusal is what
//! keeps a stress build off the internet (#234), so its edges are pinned too.

use super::{loopback_request_url, loopback_to_plain_http};

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

/// The refusal has to name the URL: it is the only clue to which library
/// request tried to leave loopback.
fn assert_refused(url: &str) {
    match loopback_request_url(url) {
        Ok(sent) => panic!("{url:?} went out as {sent:?}, expected a refusal"),
        Err(refused) => {
            assert_eq!(refused.url, url, "the refusal carries the URL");
            assert!(
                refused.to_string().contains(&format!("{url:?}")),
                "the message names {url:?}: {refused}"
            );
        }
    }
}

#[test]
fn an_off_loopback_url_is_refused_naming_it() {
    let off_loopback = [
        "https://web.whatsapp.com/sw.js",
        "http://example.com/x",
        "https://mmg.whatsapp.net/v/t62.7118-24/enc?auth=a&token=t",
    ];
    for url in off_loopback {
        assert_refused(url);
    }
}

#[test]
fn a_lookalike_or_disguised_host_is_refused() {
    let disguised = [
        "https://127.0.0.1.evil.example/x",
        "https://localhost.evil.example:443/x",
        "http://localhost.evil.example/x",
        "https://evil.example/127.0.0.1/x",
        "https://user@127.0.0.1/x",
        "http://user@127.0.0.1/x",
        "https://user@127.0.0.1.evil.example/x",
        "ftp://127.0.0.1/x",
        "127.0.0.1/x",
        "https://",
        "",
    ];
    for url in disguised {
        assert_refused(url);
    }
}

#[test]
fn a_loopback_https_url_goes_out_as_plain_http() {
    let cases = [
        (
            "https://127.0.0.1:41234/v/t62/enc?auth=a&token=t",
            "http://127.0.0.1:41234/v/t62/enc?auth=a&token=t",
        ),
        ("https://127.0.0.1/x", "http://127.0.0.1/x"),
        ("https://localhost:8080/x", "http://localhost:8080/x"),
        ("https://localhost", "http://localhost"),
        ("https://[::1]:9/x", "http://[::1]:9/x"),
    ];
    for (url, expected) in cases {
        assert_eq!(
            loopback_request_url(url).expect("a loopback URL is sent"),
            expected,
            "{url}"
        );
    }
}

#[test]
fn a_loopback_plain_http_url_goes_out_unchanged() {
    let unchanged = [
        "http://127.0.0.1:41234/x",
        "http://localhost/x",
        "http://[::1]:9/x?a=b",
    ];
    for url in unchanged {
        assert_eq!(
            loopback_request_url(url).expect("a loopback URL is sent"),
            url,
            "{url}"
        );
    }
}
