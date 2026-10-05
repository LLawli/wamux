//! Atomic tc-token writers (#93). Each is ONE statement and no transaction.

/// Advance only `sender_timestamp`, never backwards. A missing row is created
/// with an empty token and `token_timestamp = sender_timestamp`; an existing row
/// keeps its token and `token_timestamp`. The empty token is a bind (`$5`)
/// because the blob literal is spelled differently per engine, and the
/// never-backwards pick is a `CASE` for the same reason (`GREATEST` / `MAX`).
/// The placeholders appear out of order (`$1, $5, $2, $2, ...`), which is what
/// turso mis-binds without the `$N` -> `?N` rewrite (#106).
pub const TOUCH_SENDER_TIMESTAMP: &str = "INSERT INTO tc_tokens
            (jid, token, token_timestamp, sender_timestamp, device_id, updated_at)
         VALUES ($1, $5, $2, $2, $3, $4)
         ON CONFLICT (jid, device_id) DO UPDATE SET
            sender_timestamp = CASE
                WHEN COALESCE(tc_tokens.sender_timestamp, $2) > $2
                    THEN COALESCE(tc_tokens.sender_timestamp, $2)
                ELSE $2
            END,
            updated_at = EXCLUDED.updated_at";

/// Store a received token when it is newer than (or equal to) the stored one, or
/// when the stored token is empty (a row `TOUCH_SENDER_TIMESTAMP` created).
/// Never touches `sender_timestamp` of an existing row.
pub const STORE_RECEIVED: &str = "INSERT INTO tc_tokens
            (jid, token, token_timestamp, sender_timestamp, device_id, updated_at)
         VALUES ($1, $2, $3, NULL, $4, $5)
         ON CONFLICT (jid, device_id) DO UPDATE SET
            token = EXCLUDED.token,
            token_timestamp = EXCLUDED.token_timestamp,
            updated_at = EXCLUDED.updated_at
         WHERE length(tc_tokens.token) = 0
            OR EXCLUDED.token_timestamp >= tc_tokens.token_timestamp";
