//! The wamux-specific `accounts` table. `uuid` is TEXT on SQLite and Turso and
//! a UUID on Postgres, so the bind and the decode are per driver; the text is
//! shared.

pub const INSERT_ACCOUNT: &str = "INSERT INTO accounts (uuid, external_ref)
     VALUES ($1, $2)
     RETURNING uuid, external_ref, device_id, push_name, created_at";

pub const LIST_ACCOUNTS: &str = "SELECT uuid, external_ref, device_id, push_name, created_at
     FROM accounts ORDER BY device_id";

pub const DELETE_ACCOUNT: &str = "DELETE FROM accounts WHERE uuid = $1";
