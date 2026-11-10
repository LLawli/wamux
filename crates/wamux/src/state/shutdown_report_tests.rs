use wacore::store::error::StoreError;
use whatsapp_rust::DrainOutcome;

use super::shutdown_shortfalls;

const CLEAN: [(&str, DrainOutcome); 3] = [
    ("inbound", DrainOutcome::Completed),
    ("outbound", DrainOutcome::Completed),
    ("signal_settle", DrainOutcome::Completed),
];

fn store_error(text: &str) -> StoreError {
    StoreError::Database(text.into())
}

#[test]
fn a_clean_shutdown_reports_nothing() {
    assert!(shutdown_shortfalls(None, 0, None, &CLEAN).is_empty());
}

#[test]
fn a_failed_device_flush_carries_the_storage_error() {
    let error = store_error("disk full");
    let shortfalls = shutdown_shortfalls(Some(&error), 0, None, &CLEAN);
    assert_eq!(shortfalls.len(), 1);
    assert!(
        shortfalls[0].starts_with("device flush failed: "),
        "{shortfalls:?}"
    );
    assert!(shortfalls[0].contains("disk full"), "{shortfalls:?}");
}

#[test]
fn failed_secret_batches_are_counted_with_the_first_error() {
    let first = store_error("locked");
    let shortfalls = shutdown_shortfalls(None, 3, Some(&first), &CLEAN);
    assert_eq!(shortfalls.len(), 1);
    assert!(shortfalls[0].starts_with("message secrets: 3 batch(es) not written, first: "));
    assert!(shortfalls[0].contains("locked"), "{shortfalls:?}");
}

#[test]
fn every_unfinished_drain_is_named_in_order() {
    let drains = [
        ("inbound", DrainOutcome::TimedOut),
        ("outbound", DrainOutcome::Completed),
        ("signal_settle", DrainOutcome::Failed),
    ];
    assert_eq!(
        shutdown_shortfalls(None, 0, None, &drains),
        ["inbound drain TimedOut", "signal_settle drain Failed"]
    );
}

/// The run loop's cleanup owning the connection first is the normal graceful
/// stop, measured on live accounts; the library logs the real causes itself.
#[test]
fn an_unobserved_drain_is_not_a_shortfall() {
    let drains = [
        ("inbound", DrainOutcome::Completed),
        ("outbound", DrainOutcome::Completed),
        ("signal_settle", DrainOutcome::Unobserved),
    ];
    assert!(shutdown_shortfalls(None, 0, None, &drains).is_empty());
}
