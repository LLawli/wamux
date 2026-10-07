//! Times as the contract carries them (#132): the server counts in seconds and
//! as `u64`; every timestamp in this contract is `int64` milliseconds.

/// Seconds into milliseconds. Saturating, so a nonsense value cannot come back
/// as a date in the past.
pub(crate) fn millis_from_seconds(seconds: u64) -> i64 {
    saturating_i64(seconds).saturating_mul(1000)
}

/// Seconds into milliseconds for a signed wire value (#134). Saturating, for
/// the same reason as `millis_from_seconds`.
pub(crate) fn millis_from_signed_seconds(seconds: i64) -> i64 {
    seconds.saturating_mul(1000)
}

/// A wire `u64` into the contract's `int64`, clamped rather than wrapped into
/// a negative time.
pub(crate) fn saturating_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(test)]
#[path = "wire_time_tests.rs"]
mod tests;
