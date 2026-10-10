//! `DeviceStore::maintenance` on turso (#104, #106). Kept out of
//! `device_store.rs` for the `*_store.rs` rule.

use wacore::store::error::Result;

use super::TursoConn;
use super::exec::fetch_all_raw;
use super::turso_error::{db, is_busy};

/// Only the WAL checkpoint. `PRAGMA optimize` and `analysis_limit` are accepted
/// by turso 0.8.1 and do nothing, so running them would only look like upkeep.
/// TRUNCATE is the only mode that returns the `-wal` file's blocks to the
/// filesystem. It declines rather than blocks while a reader holds a snapshot,
/// so a skipped truncate (Busy) is the normal outcome, never a failure; an I/O
/// error or a full disk means the log could not be written back, which is what
/// this pass exists to catch, so those stay errors.
pub(super) async fn run(conn: &TursoConn) -> Result<()> {
    let guard = conn.lock_autocommit().await?;
    match fetch_all_raw(&guard, "PRAGMA wal_checkpoint(TRUNCATE)", Vec::new()).await {
        Ok(_checkpoint_row) => Ok(()),
        Err(error) if is_busy(&error) => Ok(()),
        Err(error) => Err(db(error)),
    }
}
