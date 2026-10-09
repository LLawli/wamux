//! #164: the store key is 64 hex characters in a 0600 file, and nothing about
//! it leaks into an error or a `Debug`.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use super::{StoreKey, StoreKeyError};

const HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn key_file(dir: &tempfile::TempDir, content: &str, mode: u32) -> std::path::PathBuf {
    let path = dir.path().join("store-key");
    fs::write(&path, content).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
    path
}

#[test]
fn parse_hex_accepts_64_hex_and_one_trailing_newline() {
    let plain = StoreKey::parse_hex(HEX).unwrap();
    assert_eq!(
        plain.id(),
        StoreKey::parse_hex(&format!("{HEX}\n")).unwrap().id()
    );
    assert_eq!(
        plain.id(),
        StoreKey::parse_hex(&format!("{HEX}\r\n")).unwrap().id()
    );
    let upper = StoreKey::parse_hex(&HEX.to_uppercase()).unwrap();
    assert_eq!(plain.id(), upper.id(), "case does not matter");
}

#[test]
fn parse_hex_rejects_short_long_and_non_hex_without_echoing_the_key() {
    let cases = [
        ("empty", String::new()),
        ("short", HEX[..62].to_string()),
        ("long", format!("{HEX}00")),
        ("non hex", format!("{}zz", &HEX[..62])),
        ("two newlines", format!("{HEX}\n\n")),
        ("inner space", format!("{} {}", &HEX[..32], &HEX[32..])),
    ];
    for (case, text) in cases {
        let error = StoreKey::parse_hex(&text).expect_err(case);
        assert!(
            matches!(error, StoreKeyError::BadFormat),
            "{case}: {error:?}"
        );
        let shown = format!("{error} {error:?}");
        assert!(
            !shown.contains(&HEX[..16]),
            "{case}: the message echoes the key"
        );
        assert!(shown.contains("64 hex characters"), "{case}: {shown}");
    }
}

#[test]
fn from_file_reads_a_0600_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = key_file(&dir, &format!("{HEX}\n"), 0o600);
    let key = StoreKey::from_file(&path).unwrap();
    assert_eq!(key.id(), StoreKey::parse_hex(HEX).unwrap().id());
}

#[test]
fn from_file_refuses_a_group_or_world_readable_file() {
    let dir = tempfile::tempdir().unwrap();
    for mode in [0o644, 0o640, 0o604, 0o660] {
        let path = key_file(&dir, HEX, mode);
        let error = StoreKey::from_file(&path).expect_err("loose mode");
        assert!(
            matches!(error, StoreKeyError::LooseMode { .. }),
            "{mode:o}: {error:?}"
        );
        let shown = error.to_string();
        assert!(shown.contains(&format!("{mode:o}")), "{shown}");
        assert!(shown.contains("chmod 600"), "{shown}");
        assert!(!shown.contains(&HEX[..16]), "{shown}");
    }
}

#[test]
fn from_file_names_the_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("absent");
    let error = StoreKey::from_file(&path).expect_err("no such file");
    assert!(
        matches!(error, StoreKeyError::Unreadable { .. }),
        "{error:?}"
    );
    assert!(error.to_string().contains("absent"), "{error}");
}

#[test]
fn key_id_is_stable_and_differs_between_keys() {
    let a = StoreKey::parse_hex(HEX).unwrap();
    let b = StoreKey::parse_hex(&"cd".repeat(32)).unwrap();
    assert_eq!(a.id(), StoreKey::parse_hex(HEX).unwrap().id());
    assert_ne!(a.id(), b.id());
}

#[test]
fn debug_prints_the_id_and_never_the_key() {
    let key = StoreKey::parse_hex(HEX).unwrap();
    let shown = format!("{key:?}");
    assert!(shown.starts_with("StoreKey(id="), "{shown}");
    assert!(!shown.contains(&HEX[..16]), "{shown}");
}
