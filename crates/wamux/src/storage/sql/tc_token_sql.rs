//! Atomic tc-token writers for `ProtocolStore` (#93). Each is ONE statement and no
//! transaction: the trait requires them atomic against `put_tc_token`, and the
//! default read-modify-write lost concurrent writers (history sync vs send path).
//! Split from `protocol_store.rs` to keep that file under the 500-line cap.

use wacore::store::error::Result;

use super::SqlPool;
use crate::storage::blob_codec::now_secs;

/// Advance only `sender_timestamp`, never backwards. A missing row is created
/// with an empty token and `token_timestamp = sender_timestamp`; an existing row
/// keeps its token and `token_timestamp`. The empty token is a bind (`$5`)
/// because the blob literal is spelled differently per engine, and the
/// never-backwards pick is a `CASE` for the same reason (`GREATEST` / `MAX`).
pub(super) async fn touch_sender_timestamp(
    pool: &SqlPool,
    device_id: i32,
    jid: &str,
    sender_timestamp: i64,
) -> Result<()> {
    execute_sql!(
        pool,
        "INSERT INTO tc_tokens
            (jid, token, token_timestamp, sender_timestamp, device_id, updated_at)
         VALUES ($1, $5, $2, $2, $3, $4)
         ON CONFLICT (jid, device_id) DO UPDATE SET
            sender_timestamp = CASE
                WHEN COALESCE(tc_tokens.sender_timestamp, $2) > $2
                    THEN COALESCE(tc_tokens.sender_timestamp, $2)
                ELSE $2
            END,
            updated_at = EXCLUDED.updated_at",
        jid,
        sender_timestamp,
        device_id,
        now_secs(),
        Vec::<u8>::new()
    )?;
    Ok(())
}

/// Store a received token when it is newer than (or equal to) the stored one, or
/// when the stored token is empty (a row `touch_sender_timestamp` created).
/// Never touches `sender_timestamp` of an existing row.
pub(super) async fn store_received(
    pool: &SqlPool,
    device_id: i32,
    jid: &str,
    token: &[u8],
    token_timestamp: i64,
) -> Result<()> {
    execute_sql!(
        pool,
        "INSERT INTO tc_tokens
            (jid, token, token_timestamp, sender_timestamp, device_id, updated_at)
         VALUES ($1, $2, $3, NULL, $4, $5)
         ON CONFLICT (jid, device_id) DO UPDATE SET
            token = EXCLUDED.token,
            token_timestamp = EXCLUDED.token_timestamp,
            updated_at = EXCLUDED.updated_at
         WHERE length(tc_tokens.token) = 0
            OR EXCLUDED.token_timestamp >= tc_tokens.token_timestamp",
        jid,
        token,
        token_timestamp,
        device_id,
        now_secs()
    )?;
    Ok(())
}
