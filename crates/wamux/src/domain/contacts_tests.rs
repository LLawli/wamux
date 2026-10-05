//! `contacts` (#116): the tests that lived inline, moved so the loop can seal
//! them. Unchanged: they never took an identifier.

use super::validate_push_name;
use crate::error::WamuxError;

// E2E triage 2026-06-16: an empty push name must fail fast as
// InvalidArgument (the lib rejects it pre-network with a bare anyhow that
// client_err would otherwise mislabel as Unavailable/503 at the edge).
#[test]
fn empty_push_name_is_invalid_argument() {
    let err = validate_push_name("").expect_err("empty name must be rejected");
    assert!(matches!(err, WamuxError::InvalidArgument(_)));
}

#[test]
fn non_empty_push_name_is_accepted() {
    assert!(validate_push_name("Ana").is_ok());
    // A single space is non-empty: the core mirrors the lib's empty-only
    // rule and leaves any trim/blank policy to the edge.
    assert!(validate_push_name(" ").is_ok());
}
