//! #166: the rotation plan. It accepts only a whole encrypted store and its own
//! key, refuses a new key equal to the old one, and the mark it writes opens
//! with the new key and not with the old.

use super::*;

fn key(digit: &str) -> StoreKey {
    StoreKey::parse_hex(&digit.repeat(32)).unwrap()
}

fn encrypted_under(key: &StoreKey) -> StoredEncryption {
    let mark = new_encrypted_store(key).unwrap().mark.unwrap();
    StoredEncryption {
        encrypted: true,
        key_id: Some(mark.key_id.to_vec()),
        verifier: Some(mark.verifier),
    }
}

fn refusal(result: StoreResult<Rotation>) -> String {
    match result {
        Ok(_) => panic!("the rotation was planned, a refusal was expected"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn rotation_marks_the_new_key_and_only_the_new_key_opens_the_new_mark() {
    let (old, new) = (key("ab"), key("cd"));
    let rotation = plan_rotation(&encrypted_under(&old), false, &old, &new).unwrap();
    assert_eq!(rotation.mark.key_id, new.id());

    let sealed_by_old = BlobCipher::sealing(&old)
        .seal_at(3, "sessions", "record", b"alice", b"secret")
        .unwrap();
    let opened = rotation
        .from
        .open_at(3, "sessions", "record", b"alice", &sealed_by_old)
        .unwrap();
    assert_eq!(opened, b"secret");
    let resealed = rotation
        .to
        .seal_at(3, "sessions", "record", b"alice", &opened)
        .unwrap();
    assert_eq!(resealed[1..5], new.id(), "sealed under the new key id");

    let rotated = StoredEncryption {
        encrypted: true,
        key_id: Some(rotation.mark.key_id.to_vec()),
        verifier: Some(rotation.mark.verifier),
    };
    let reopened = resolve(&rotated, true, false, Some(&new)).unwrap();
    assert!(reopened.cipher.is_sealing() && reopened.mark.is_none() && !reopened.convert);
    let with_old = resolve(&rotated, true, false, Some(&old)).err().unwrap();
    assert!(with_old.to_string().contains("does not match"));
}

#[test]
fn rotation_refuses_an_old_key_that_does_not_match() {
    let stored = encrypted_under(&key("ab"));
    let shown = refusal(plan_rotation(&stored, false, &key("ef"), &key("cd")));
    assert!(shown.contains("does not match"), "{shown}");
}

#[test]
fn rotation_refuses_a_new_key_equal_to_the_old() {
    let old = key("ab");
    let shown = refusal(plan_rotation(
        &encrypted_under(&old),
        false,
        &old,
        &key("ab"),
    ));
    assert!(shown.contains("already"), "{shown}");
}

#[test]
fn rotation_refuses_a_plaintext_store_and_a_half_converted_one() {
    let plaintext = StoredEncryption::from_row("plaintext", None, None).unwrap();
    let half_converted = StoredEncryption {
        encrypted: false,
        ..encrypted_under(&key("ab"))
    };
    for (stored, has_progress) in [(plaintext, false), (half_converted, true)] {
        let shown = refusal(plan_rotation(&stored, has_progress, &key("ab"), &key("cd")));
        assert!(shown.contains("not encrypted"), "{shown}");
    }
}

#[test]
fn rotation_refuses_a_half_finished_decrypt() {
    let old = key("ab");
    let shown = refusal(plan_rotation(
        &encrypted_under(&old),
        true,
        &old,
        &key("cd"),
    ));
    assert!(shown.contains("wamux store decrypt"), "{shown}");
}
