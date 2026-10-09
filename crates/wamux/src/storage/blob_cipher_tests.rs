//! #164: a sealed blob opens only for the key and the place it was sealed for.

use super::{BlobCipher, BlobContext};
use crate::storage::store_key::StoreKey;

fn key(byte: &str) -> StoreKey {
    StoreKey::parse_hex(&byte.repeat(32)).unwrap()
}

fn context<'a>(row: &'a [u8]) -> BlobContext<'a> {
    BlobContext {
        device_id: 7,
        table: "sessions",
        column: "record",
        row,
    }
}

const HEADER: usize = 1 + 4 + 24;
const TAG: usize = 16;

#[test]
fn seal_open_round_trips() {
    let cipher = BlobCipher::sealing(&key("ab"));
    let ctx = context(b"alice@s.whatsapp.net");
    let sealed = cipher.seal(&ctx, b"a session record").unwrap();
    assert_eq!(sealed.len(), HEADER + b"a session record".len() + TAG);
    assert_eq!(sealed[0], 1, "version");
    assert_eq!(sealed[1..5], key("ab").id(), "key id");
    assert!(
        !sealed.windows(7).any(|w| w == b"session"),
        "no plaintext in the blob"
    );
    assert_eq!(cipher.open(&ctx, &sealed).unwrap(), b"a session record");
}

#[test]
fn seal_uses_a_fresh_nonce_each_time() {
    let cipher = BlobCipher::sealing(&key("ab"));
    let ctx = context(b"row");
    let first = cipher.seal(&ctx, b"same").unwrap();
    let second = cipher.seal(&ctx, b"same").unwrap();
    assert_ne!(first[5..29], second[5..29], "nonce");
    assert_ne!(first, second);
}

#[test]
fn empty_stays_empty_both_ways() {
    let cipher = BlobCipher::sealing(&key("ab"));
    let ctx = context(b"row");
    assert!(cipher.seal(&ctx, b"").unwrap().is_empty());
    assert!(cipher.open(&ctx, b"").unwrap().is_empty());
}

#[test]
fn open_fails_for_a_wrong_key() {
    let ctx = context(b"row");
    let sealed = BlobCipher::sealing(&key("ab"))
        .seal(&ctx, b"secret")
        .unwrap();
    let error = BlobCipher::sealing(&key("cd"))
        .open(&ctx, &sealed)
        .expect_err("wrong key");
    assert!(!error.to_string().is_empty());
}

#[test]
fn open_fails_when_the_device_table_column_or_row_differs() {
    let cipher = BlobCipher::sealing(&key("ab"));
    let home = context(b"alice@s.whatsapp.net");
    let sealed = cipher.seal(&home, b"secret").unwrap();
    let elsewhere = [
        (
            "device",
            BlobContext {
                device_id: 8,
                ..home
            },
        ),
        (
            "table",
            BlobContext {
                table: "sender_keys",
                ..home
            },
        ),
        (
            "column",
            BlobContext {
                column: "other",
                ..home
            },
        ),
        ("row", context(b"bob@s.whatsapp.net")),
    ];
    for (case, ctx) in elsewhere {
        assert!(
            cipher.open(&ctx, &sealed).is_err(),
            "{case} moved but still opened"
        );
    }
    assert_eq!(
        cipher.open(&home, &sealed).unwrap(),
        b"secret",
        "the home still opens"
    );
}

#[test]
fn open_fails_on_a_truncated_or_unknown_version_blob() {
    let cipher = BlobCipher::sealing(&key("ab"));
    let ctx = context(b"row");
    let sealed = cipher.seal(&ctx, b"secret").unwrap();
    let mut bad_version = sealed.clone();
    bad_version[0] = 9;
    let mut flipped_tag = sealed.clone();
    *flipped_tag.last_mut().unwrap() ^= 1;
    let cases = [
        ("one byte", vec![1u8]),
        ("header only", sealed[..HEADER].to_vec()),
        ("cut tag", sealed[..sealed.len() - 1].to_vec()),
        ("version 9", bad_version),
        ("flipped tag", flipped_tag),
        ("plaintext bytes", b"plain, never sealed".to_vec()),
    ];
    for (case, blob) in cases {
        assert!(cipher.open(&ctx, &blob).is_err(), "{case} opened");
    }
}

#[test]
fn passthrough_returns_the_bytes_untouched() {
    let cipher = BlobCipher::passthrough();
    let ctx = context(b"row");
    assert!(!cipher.is_sealing());
    assert_eq!(cipher.seal(&ctx, b"as is").unwrap(), b"as is");
    assert_eq!(cipher.open(&ctx, b"as is").unwrap(), b"as is");
    assert!(BlobCipher::sealing(&key("ab")).is_sealing());
}
