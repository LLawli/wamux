//! #79: the global spool budget. Reservations are held until dropped, refused
//! with `ResourceExhausted` when they would pass the maximum, and 0 is no
//! maximum at all.

use super::SpoolBudget;
use crate::error::WamuxError;

const MIB: u64 = 1024 * 1024;

fn assert_exhausted(result: Result<impl std::fmt::Debug, WamuxError>) {
    match result {
        Err(WamuxError::ResourceExhausted(message)) => {
            assert_eq!(message, "media spool budget exhausted");
        }
        other => panic!("expected ResourceExhausted, got {other:?}"),
    }
}

#[test]
fn a_reservation_within_the_budget_is_granted() {
    let budget = SpoolBudget::new(10 * MIB);
    let first = budget.try_reserve(4 * MIB).expect("4 of 10");
    let second = budget.try_reserve(6 * MIB).expect("exactly the rest");
    assert_eq!(first.bytes(), 4 * MIB);
    assert_eq!(second.bytes(), 6 * MIB);
    assert_eq!(budget.in_flight(), 10 * MIB);
}

#[test]
fn a_reservation_past_the_budget_is_resource_exhausted() {
    let budget = SpoolBudget::new(10 * MIB);
    let _held = budget.try_reserve(8 * MIB).expect("8 of 10");
    assert_exhausted(budget.try_reserve(3 * MIB));
    assert_eq!(budget.in_flight(), 8 * MIB, "a refusal holds nothing");
    assert_exhausted(SpoolBudget::new(MIB).try_reserve(MIB + 1));
}

#[test]
fn dropping_a_reservation_releases_it() {
    let budget = SpoolBudget::new(10 * MIB);
    let held = budget.try_reserve(10 * MIB).expect("all of it");
    assert_exhausted(budget.try_reserve(1));
    drop(held);
    assert_eq!(budget.in_flight(), 0);
    budget.try_reserve(10 * MIB).expect("all of it again");
}

#[test]
fn zero_means_unlimited() {
    let budget = SpoolBudget::new(0);
    let a = budget.try_reserve(u64::MAX / 2).expect("no maximum");
    let mut b = budget.try_reserve(1024 * MIB).expect("still none");
    b.try_grow(1024 * MIB).expect("growing has none either");
    assert_eq!(a.bytes() + b.bytes(), u64::MAX / 2 + 2048 * MIB);
}

#[test]
fn try_grow_past_the_budget_fails_and_keeps_what_was_held() {
    let budget = SpoolBudget::new(10 * MIB);
    let mut growing = budget.try_reserve(0).expect("an empty start");
    growing.try_grow(6 * MIB).expect("6 of 10");
    growing.try_grow(4 * MIB).expect("exactly the rest");
    assert_exhausted(growing.try_grow(1));
    assert_eq!(growing.bytes(), 10 * MIB, "the refusal kept what was held");
    assert_eq!(budget.in_flight(), 10 * MIB);
    drop(growing);
    assert_eq!(budget.in_flight(), 0, "the grown bytes are released too");
}
