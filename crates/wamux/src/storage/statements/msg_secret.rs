//! `MsgSecretStore` statements.

/// Upsert must never SHORTEN a retention window: `expires_at = 0` means
/// "never" and beats any deadline, otherwise the later deadline wins. The
/// same guard applies to `message_ts`, where a `0` ("unknown") must not
/// clobber a parent time we already learned. Both rules are the lib's
/// `merge_msg_secret_*` helpers, expressed in SQL so one statement per row
/// stays atomic against a concurrent redelivery. The later-wins pick is a
/// `CASE` because `GREATEST` (Postgres) and two-argument `MAX` (SQLite) do
/// not spell the same way; all the columns involved are NOT NULL.
pub const PUT_MSG_SECRET: &str = "INSERT INTO msg_secrets
                     (chat, sender, msg_id, secret, expires_at, message_ts, device_id)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT (chat, sender, msg_id, device_id) DO UPDATE SET
                     secret     = EXCLUDED.secret,
                     expires_at = CASE
                         WHEN msg_secrets.expires_at = 0 OR EXCLUDED.expires_at = 0 THEN 0
                         WHEN msg_secrets.expires_at > EXCLUDED.expires_at
                             THEN msg_secrets.expires_at
                         ELSE EXCLUDED.expires_at
                     END,
                     message_ts = CASE
                         WHEN msg_secrets.message_ts > EXCLUDED.message_ts
                             THEN msg_secrets.message_ts
                         ELSE EXCLUDED.message_ts
                     END";

pub const GET_MSG_SECRET: &str = "SELECT secret FROM msg_secrets
             WHERE chat = $1 AND sender = $2 AND msg_id = $3 AND device_id = $4";

pub const GET_MSG_SECRET_WITH_TS: &str = "SELECT secret, message_ts FROM msg_secrets
             WHERE chat = $1 AND sender = $2 AND msg_id = $3 AND device_id = $4";

/// `expires_at = 0` is "never" and is deliberately excluded from the sweep.
pub const DELETE_EXPIRED_MSG_SECRETS: &str = "DELETE FROM msg_secrets
             WHERE expires_at <> 0 AND expires_at <= $1 AND device_id = $2";
