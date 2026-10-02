//! Engine parity for every method the two stores implement (#60).
//!
//! The Postgres and SQLite stores each implement 59 methods of the wacore
//! traits. This is the crypto-critical state of every account, and #65 is about
//! to rewrite all of it: these tests are the safety net that rewrite runs
//! against. A divergence in sender keys, base keys or tc-tokens would otherwise
//! show up only as a failed decrypt on a live account.
//!
//! Shape: one generic body per behavior, run as a `postgres_<name>` /
//! `sqlite_<name>` pair asserting the same constants, so passing on both IS the
//! parity claim. The `sqlite_` half needs no database and runs in
//! `scripts/ci.sh --no-postgres`. The `both_engines_*` tests in `blobs` need
//! both engines at once and compare the raw stored bytes.
//!
//! `scripts/check-store-coverage.py` fails CI if a store method stops being
//! called from `crates/wamux/tests/`.

// Only a subset of the shared helpers is used by this binary.
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;

mod app_sync;
mod blobs;
mod harness;
mod msg_secret;
mod protocol;
mod protocol_cache;
mod signal;
