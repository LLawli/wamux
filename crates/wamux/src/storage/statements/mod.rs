//! Every store statement, written once (#106). Both engine families run this
//! text: `sql` hands it to sqlx as is, `turso` rewrites `$N` to `?N` first
//! (`turso::placeholders`) because turso binds `$N` by order of appearance,
//! not by number. A repo check keeps it that way: no SQL literal in a family,
//! and only `$N` here.
//!
//! One module per store trait. The statements with no common form across the
//! drivers (Postgres `= ANY($2)` arrays, `FOR UPDATE`) stay beside the Postgres
//! code in `sql`; their SQLite shape is here, and is what Turso runs.

pub mod accounts;
pub mod app_sync;
pub mod bincode_upgrade;
pub mod device;
pub mod msg_secret;
pub mod protocol;
pub mod signal;
pub mod tc_token;

/// The readiness probe: proves a connection can be had and answers.
pub const PING: &str = "SELECT 1";
