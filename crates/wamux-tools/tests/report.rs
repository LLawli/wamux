//! #64: the three check outcomes and the exit code they add up to.

use std::process::ExitCode;

use wamux_tools::report::{Check, Counts, Outcome, Report, check_line};

#[test]
fn an_empty_report_is_not_green() {
    let report = Report::new();
    assert!(
        !report.is_green(),
        "a run that checked nothing proved nothing"
    );
    assert_eq!(report.exit_code(), ExitCode::FAILURE);
}

#[test]
fn a_report_of_only_accepted_calls_is_not_green() {
    let mut report = Report::new();
    report.accepted("Messaging.SendPresence", "ok");
    report.accepted("Messaging.MarkRead", "ok");
    assert!(!report.is_green());
    assert_eq!(report.exit_code(), ExitCode::FAILURE);
}

#[test]
fn one_failure_makes_the_report_red() {
    let mut report = Report::new();
    report.pass("Account.CreateAccount", "uuid=x");
    report.fail("Messaging.SendText", "no delivered receipt in 30s");
    assert!(!report.is_green());
    assert_eq!(report.exit_code(), ExitCode::FAILURE);
}

#[test]
fn passes_plus_accepted_with_no_failure_is_green() {
    let mut report = Report::new();
    report.pass("Account.CreateAccount", "uuid=x");
    report.accepted("Messaging.SendPresence", "ok");
    assert!(report.is_green());
    assert_eq!(report.exit_code(), ExitCode::SUCCESS);
}

#[test]
fn verify_maps_the_condition_to_pass_or_fail() {
    let mut report = Report::new();
    report.verify("a", true, "held");
    report.verify("b", false, "did not hold");
    let outcomes: Vec<Outcome> = report.checks().iter().map(|c| c.outcome).collect();
    assert_eq!(outcomes, vec![Outcome::Pass, Outcome::Fail]);
}

#[test]
fn accepted_rpc_records_ok_as_accepted_and_hands_the_value_back() {
    let mut report = Report::new();
    let value = report.accepted_rpc("Contact.SubscribePresence", Ok::<u8, tonic::Status>(7));
    assert_eq!(value, Some(7));
    assert_eq!(report.checks()[0].outcome, Outcome::Accepted);
}

#[test]
fn accepted_rpc_records_an_error_as_fail_with_its_message() {
    let mut report = Report::new();
    let value = report.accepted_rpc::<u8>(
        "Messaging.MarkRead",
        Err(tonic::Status::not_found("no such chat")),
    );
    assert_eq!(value, None);
    let check = &report.checks()[0];
    assert_eq!(check.outcome, Outcome::Fail);
    assert!(check.detail.contains("no such chat"), "{check:?}");
}

#[test]
fn counts_add_up_each_outcome() {
    let mut report = Report::new();
    report.pass("a", "");
    report.pass("b", "");
    report.accepted("c", "");
    report.fail("d", "");
    assert_eq!(
        report.counts(),
        Counts {
            pass: 2,
            accepted: 1,
            fail: 1
        }
    );
}

#[test]
fn checks_keep_their_name_and_detail_in_order() {
    let mut report = Report::new();
    report.pass("first", "one");
    report.fail("second", "two");
    let names: Vec<&str> = report.checks().iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["first", "second"]);
    assert_eq!(report.checks()[1].detail, "two");
}

#[test]
fn check_lines_tag_each_outcome() {
    let line = |outcome| {
        check_line(&Check {
            name: "Messaging.SendText".into(),
            outcome,
            detail: "id=ABC".into(),
        })
    };
    assert_eq!(line(Outcome::Pass), "[PASS] Messaging.SendText :: id=ABC");
    assert_eq!(
        line(Outcome::Accepted),
        "[ACCEPTED] Messaging.SendText :: id=ABC"
    );
    assert_eq!(line(Outcome::Fail), "[FAIL] Messaging.SendText :: id=ABC");
}

#[test]
fn the_summary_line_counts_every_outcome() {
    let mut report = Report::new();
    report.pass("a", "");
    report.accepted("b", "");
    report.accepted("c", "");
    report.fail("d", "");
    assert_eq!(report.summary_line(), "summary: 1 pass, 2 accepted, 1 fail");
}

#[test]
fn finish_returns_the_same_exit_code() {
    let mut report = Report::new();
    report.fail("a", "");
    assert_eq!(report.finish(), ExitCode::FAILURE);
    let mut report = Report::new();
    report.pass("a", "");
    assert_eq!(report.finish(), ExitCode::SUCCESS);
}
