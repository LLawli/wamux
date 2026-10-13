//! #77: who the `SO_PEERCRED` allowlist admits. The peer is `(uid, gid)`.

use super::PeerAllowlist;

#[test]
fn empty_lists_admit_everyone() {
    let open = PeerAllowlist::new(vec![], vec![]);
    assert!(open.admits(Some((1000, 1000))));
    assert!(open.admits(Some((0, 0))));
    assert!(
        open.admits(None),
        "without a list the socket's mode is the gate"
    );
}

#[test]
fn a_listed_uid_is_admitted() {
    let list = PeerAllowlist::new(vec![1000], vec![]);
    assert!(list.admits(Some((1000, 4242))));
    assert!(!list.admits(Some((1001, 4242))));
}

#[test]
fn a_listed_gid_is_admitted() {
    let list = PeerAllowlist::new(vec![], vec![967]);
    assert!(list.admits(Some((1001, 967))));
    assert!(
        !list.admits(Some((967, 1000))),
        "a uid equal to a listed gid number is not a match"
    );
}

#[test]
fn an_unlisted_peer_is_denied() {
    let list = PeerAllowlist::new(vec![1000], vec![967]);
    assert!(!list.admits(Some((1001, 1001))));
    assert!(!list.admits(Some((0, 0))), "not even root is implicit");
    let daemon_uid = nix::unistd::getuid().as_raw();
    let without_us = PeerAllowlist::new(vec![daemon_uid.wrapping_add(1)], vec![]);
    assert!(
        !without_us.admits(Some((daemon_uid, 4242))),
        "the daemon's own uid is not implicit either"
    );
}

#[test]
fn a_peer_without_credentials_is_denied_when_a_list_is_set() {
    assert!(!PeerAllowlist::new(vec![1000], vec![]).admits(None));
    assert!(!PeerAllowlist::new(vec![], vec![967]).admits(None));
}
