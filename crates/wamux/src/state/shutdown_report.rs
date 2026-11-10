//! What the library's terminal shutdown left undone, logged at the boundary.
//!
//! `Client::shutdown` (and `logout`, which runs it) returns a `ShutdownReport`
//! instead of failing: the session is torn down whatever it says, so `stop`
//! and `Logout` succeed either way. A device snapshot that did not flush means
//! the next start loads older device state; failed message-secret batches mean
//! a later poll vote or reaction may not decrypt. The operator needs to see
//! both, so they are logged here, once per shutdown, never returned.

use uuid::Uuid;
use wacore::store::error::StoreError;
use wamux_types::error::cause_chain;
use whatsapp_rust::{DrainOutcome, ShutdownReport};

/// Log one warn line when `report` shows unfinished work; nothing when clean.
pub fn log_shutdown_report(account: Uuid, report: &ShutdownReport) {
    let drains = [
        ("inbound", report.inbound),
        ("outbound", report.outbound),
        ("signal_settle", report.signal_settle),
    ];
    let secret_error = report.message_secrets.first_error.as_deref();
    let shortfalls = shutdown_shortfalls(
        report.device.as_ref().err(),
        report.message_secrets.failed_batches,
        secret_error,
        &drains,
    );
    if !shortfalls.is_empty() {
        tracing::warn!(account = %account, shortfalls = %shortfalls.join("; "), "shutdown left work unfinished");
    }
}

/// One entry per unfinished part, in report order. Parts of the report as
/// arguments, because the library's report is `#[non_exhaustive]` and cannot
/// be built outside it, so a test could not reach a function taking it whole.
pub fn shutdown_shortfalls(
    device: Option<&StoreError>,
    secret_failed_batches: u64,
    secret_error: Option<&StoreError>,
    drains: &[(&str, DrainOutcome)],
) -> Vec<String> {
    let mut shortfalls: Vec<String> = Vec::new();
    if let Some(error) = device {
        shortfalls.push(format!("device flush failed: {}", cause_chain(error)));
    }
    if secret_failed_batches > 0 {
        let first = secret_error
            .map(|e| format!(", first: {}", cause_chain(e)))
            .unwrap_or_default();
        shortfalls.push(format!(
            "message secrets: {secret_failed_batches} batch(es) not written{first}"
        ));
    }
    for (part, outcome) in drains {
        if drain_unfinished(*outcome) {
            shortfalls.push(format!("{part} drain {outcome:?}"));
        }
    }
    shortfalls
}

/// `Failed` and `TimedOut` mean state may be left for redelivery. `Unobserved`
/// does not: on a graceful stop the run loop's own connection cleanup can get
/// there first, and the shutdown's cleanup then reports it never owned the
/// target (seen on a SIGTERM stop of a live account; the library logs that case
/// at debug only). It logs the two real causes of `Unobserved` at error itself,
/// so warning here would only repeat them or cry wolf.
fn drain_unfinished(outcome: DrainOutcome) -> bool {
    !matches!(outcome, DrainOutcome::Completed | DrainOutcome::Unobserved)
}

#[cfg(test)]
#[path = "shutdown_report_tests.rs"]
mod tests;
