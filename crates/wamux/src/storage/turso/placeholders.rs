//! `$N` -> `?N` (#106). The shared statements (`storage::statements`) spell
//! placeholders `$N`, which sqlx binds by number on both its drivers. turso
//! 0.8.1 binds `$N` by order of FIRST APPEARANCE, so a statement with out-of-order
//! or repeated placeholders (the tc-token upsert) writes each value into the
//! wrong column and still returns Ok. `?N` binds by number. See
//! `placeholders_tests`, which pins both behaviors.
//!
//! The rewrite runs per call, not cached by text: a statement is a few hundred
//! bytes (the 100-wide `IN` lists, about 2 KB) scanned once, which is noise next
//! to the write to disk, and a cache keyed by `&'static str` would need a global
//! map and a lock to save that scan. turso keeps its own prepared-statement
//! cache by text, and the rewritten text is identical on every call.

/// `$N` -> `?N` throughout one statement. Every `$` in the shared statement
/// text starts a placeholder (a repo check keeps `statements/` that way), so a
/// plain replace is exact.
pub(crate) fn turso_placeholders(sql: &str) -> String {
    sql.replace('$', "?")
}
