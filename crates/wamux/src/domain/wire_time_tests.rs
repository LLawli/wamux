//! #132: the seconds-to-milliseconds conversion shared by the group and the
//! channel projections.

use super::*;

#[test]
fn millis_from_seconds_multiplies_and_saturates() {
    assert_eq!(millis_from_seconds(0), 0);
    assert_eq!(millis_from_seconds(1_790_947_599), 1_790_947_599_000);
    // Past i64::MAX / 1000 the product would wrap; it clamps instead.
    assert_eq!(millis_from_seconds(i64::MAX as u64 / 1000 + 1), i64::MAX);
    assert_eq!(millis_from_seconds(u64::MAX), i64::MAX);
}

#[test]
fn saturating_i64_clamps_instead_of_wrapping() {
    assert_eq!(saturating_i64(0), 0);
    assert_eq!(saturating_i64(i64::MAX as u64), i64::MAX);
    assert_eq!(saturating_i64(u64::MAX), i64::MAX);
}
