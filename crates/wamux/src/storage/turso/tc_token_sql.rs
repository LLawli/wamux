//! Atomic tc-token writers for `ProtocolStore` on turso (#93, #106). Each is ONE
//! statement and no transaction: the trait requires them atomic against
//! `put_tc_token`. The statements repeat and reorder their placeholders, which
//! is exactly what the `$N` -> `?N` rewrite exists for.

use wacore::store::error::Result;

use super::TursoConn;
use super::exec::binds;
use crate::storage::blob_codec::now_secs;
use crate::storage::statements::tc_token::{STORE_RECEIVED, TOUCH_SENDER_TIMESTAMP};

/// Advance only `sender_timestamp`, never backwards (see the statement). The
/// empty token is a bind: the blob literal is spelled per engine.
pub(super) async fn touch_sender_timestamp(
    conn: &TursoConn,
    device_id: i32,
    jid: &str,
    sender_timestamp: i64,
) -> Result<()> {
    let empty_token: &[u8] = &[];
    let binds = binds![jid, sender_timestamp, device_id, now_secs(), empty_token];
    conn.execute(TOUCH_SENDER_TIMESTAMP, binds).await?;
    Ok(())
}

/// Store a received token when it is newer than (or equal to) the stored one, or
/// when the stored token is empty. Never touches `sender_timestamp` of an
/// existing row.
pub(super) async fn store_received(
    conn: &TursoConn,
    device_id: i32,
    jid: &str,
    token: &[u8],
    token_timestamp: i64,
) -> Result<()> {
    let binds = binds![jid, token, token_timestamp, device_id, now_secs()];
    conn.execute(STORE_RECEIVED, binds).await?;
    Ok(())
}
