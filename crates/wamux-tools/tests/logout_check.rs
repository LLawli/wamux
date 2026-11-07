//! #257: the verdict `logout_e2e` gives on the state read after `Logout`. The
//! binary itself needs a device it can unlink, so the decision is tested here.

use wamux::proto::v1 as pb;
use wamux_tools::logout_check::record_post_logout_state;
use wamux_tools::report::{Check, Outcome, Report};

fn verdict(state: i32) -> Check {
    let mut report = Report::new();
    record_post_logout_state(&mut report, state);
    let checks = report.checks();
    assert_eq!(checks.len(), 1, "one state, one check: {checks:?}");
    checks[0].clone()
}

#[test]
fn logged_out_is_a_pass() {
    let check = verdict(pb::ConnectionState::LoggedOut as i32);
    assert_eq!(check.outcome, Outcome::Pass, "{check:?}");
}

/// A logout that did not reach LOGGED_OUT must not pass, and the line must say
/// what was read and what was wanted, because a run costs a phone scan.
#[test]
fn disconnected_is_a_failure_that_names_both_states() {
    let check = verdict(pb::ConnectionState::Disconnected as i32);
    assert_eq!(check.outcome, Outcome::Fail, "{check:?}");
    assert!(check.detail.contains("Disconnected"), "{check:?}");
    assert!(check.detail.contains("LoggedOut"), "{check:?}");
}

#[test]
fn an_unknown_state_is_a_failure_that_shows_the_number() {
    let check = verdict(99);
    assert_eq!(check.outcome, Outcome::Fail, "{check:?}");
    assert!(check.detail.contains("99"), "{check:?}");
}
