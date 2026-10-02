//! Atomic tc-token writers for `ProtocolStore` (#93). Each is ONE statement and no
//! transaction: the trait requires them atomic against `put_tc_token`, and the
//! default read-modify-write lost concurrent writers (history sync vs send path).
//! Split from `protocol_store.rs` to keep that file under the 500-line cap.

use sqlx::PgPool;
use wacore::store::error::Result;

use super::now_secs;
use crate::storage::sqlx_error::db;

/// Advance only `sender_timestamp`, never backwards. A missing row is created
/// with an empty token and `token_timestamp = sender_timestamp`; an existing row
/// keeps its token and `token_timestamp`.
pub(super) async fn touch_sender_timestamp(
    pool: &PgPool,
    device_id: i32,
    jid: &str,
    sender_timestamp: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO tc_tokens
            (jid, token, token_timestamp, sender_timestamp, device_id, updated_at)
         VALUES ($1, ''::bytea, $2, $2, $3, $4)
         ON CONFLICT (jid, device_id) DO UPDATE SET
            sender_timestamp = GREATEST(COALESCE(tc_tokens.sender_timestamp, $2), $2),
            updated_at = EXCLUDED.updated_at",
    )
    .bind(jid)
    .bind(sender_timestamp)
    .bind(device_id)
    .bind(now_secs())
    .execute(pool)
    .await
    .map_err(db)?;
    Ok(())
}

/// Store a received token when it is newer than (or equal to) the stored one, or
/// when the stored token is empty (a row `touch_sender_timestamp` created).
/// Never touches `sender_timestamp` of an existing row.
pub(super) async fn store_received(
    pool: &PgPool,
    device_id: i32,
    jid: &str,
    token: &[u8],
    token_timestamp: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO tc_tokens
            (jid, token, token_timestamp, sender_timestamp, device_id, updated_at)
         VALUES ($1, $2, $3, NULL, $4, $5)
         ON CONFLICT (jid, device_id) DO UPDATE SET
            token = EXCLUDED.token,
            token_timestamp = EXCLUDED.token_timestamp,
            updated_at = EXCLUDED.updated_at
         WHERE octet_length(tc_tokens.token) = 0
            OR EXCLUDED.token_timestamp >= tc_tokens.token_timestamp",
    )
    .bind(jid)
    .bind(token)
    .bind(token_timestamp)
    .bind(device_id)
    .bind(now_secs())
    .execute(pool)
    .await
    .map_err(db)?;
    Ok(())
}
