//! Request validation for the channel poll vote (issue #26). The library's
//! own checks answer `InvalidRequest`, which `client_err` turns into
//! Unavailable, so every malformed request must stop here as InvalidArgument.

use super::*;

const CHANNEL: &str = "120363144038483540@newsletter";

/// sha256("🫠 Just this."), the option voted on live in #26 (2026-09-25).
fn just_this() -> Vec<u8> {
    wacore::poll::compute_option_hash("\u{1FAE0} Just this.").to_vec()
}

fn invalid_argument(result: Result<impl std::fmt::Debug, WamuxError>) -> String {
    match result {
        Err(WamuxError::InvalidArgument(message)) => message,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

#[test]
fn the_live_vote_hash_is_the_one_measured_on_the_wire() {
    let hex: String = just_this().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        hex,
        "107f7671b96fcff7c2524b29a031dfb5c0c6ea19c9cba6e47de723e5c6983c83"
    );
}

#[test]
fn hashes_convert_in_order() {
    let other = wacore::poll::compute_option_hash("other").to_vec();
    let hashes = option_hashes(&[just_this(), other.clone()]).expect("two distinct hashes");
    assert_eq!(hashes.len(), 2);
    assert_eq!(hashes[0].to_vec(), just_this());
    assert_eq!(hashes[1].to_vec(), other);
}

// An empty selection is how a vote is removed, not a malformed request.
#[test]
fn an_empty_selection_is_valid() {
    assert!(
        option_hashes(&[])
            .expect("empty removes the vote")
            .is_empty()
    );
}

#[test]
fn a_hash_that_is_not_32_bytes_is_refused_with_its_index() {
    let message = invalid_argument(option_hashes(&[just_this(), vec![0u8; 31]]));
    assert!(message.contains("option_hashes[1]"), "{message}");
    assert!(message.contains("got 31"), "{message}");
}

#[test]
fn a_repeated_option_is_refused() {
    let message = invalid_argument(option_hashes(&[just_this(), just_this()]));
    assert!(message.contains("option_hashes[1] repeats"), "{message}");
}

#[test]
fn more_than_the_ceiling_is_refused_and_the_ceiling_itself_passes() {
    let distinct = |n: usize| -> Vec<Vec<u8>> {
        (0..n)
            .map(|i| wacore::poll::compute_option_hash(&i.to_string()).to_vec())
            .collect()
    };
    assert_eq!(
        option_hashes(&distinct(MAX_POLL_VOTE_OPTIONS))
            .expect("the ceiling is allowed")
            .len(),
        MAX_POLL_VOTE_OPTIONS
    );
    let message = invalid_argument(option_hashes(&distinct(MAX_POLL_VOTE_OPTIONS + 1)));
    assert!(message.contains("got 1001"), "{message}");
}

#[test]
fn a_channel_jid_is_accepted() {
    assert!(require_newsletter_jid(CHANNEL).is_ok());
}

#[test]
fn a_jid_off_the_newsletter_server_is_refused() {
    for jid in ["5511999000111@s.whatsapp.net", "120363041234567890@g.us"] {
        let message = invalid_argument(require_newsletter_jid(jid));
        assert!(message.contains("not a channel"), "{jid}: {message}");
    }
}
