//! The one-row `store_encryption` marker (#164): whether the secret columns
//! hold sealed blobs, under which key, plus the verifier that catches a wrong
//! key at open time.

pub const SELECT_STATE: &str = "SELECT state, key_id, verifier FROM store_encryption WHERE id = 1";

pub const HAS_ACCOUNTS: &str = "SELECT EXISTS(SELECT 1 FROM accounts)";

/// Flip a still-plaintext, still-empty store to encrypted. One statement, so the
/// "no accounts" check and the flip cannot be split by an `INSERT` between them;
/// zero rows affected means the store was not new after all.
pub const MARK_ENCRYPTED: &str = "UPDATE store_encryption
     SET state = 'encrypted', key_id = $1, verifier = $2
     WHERE id = 1 AND state = 'plaintext' AND NOT EXISTS (SELECT 1 FROM accounts)";

pub const HAS_PROGRESS: &str = "SELECT EXISTS(SELECT 1 FROM store_conversion_progress)";

/// Record the key of a conversion BEFORE the first account is sealed (#165), so
/// a different key on the retry fails against the verifier instead of sealing
/// the rest of the accounts under a second key. The state stays 'plaintext'
/// until the last account is done; a verifier next to it means "converting".
pub const MARK_CONVERTING: &str = "UPDATE store_encryption
     SET key_id = $1, verifier = $2
     WHERE id = 1 AND state = 'plaintext' AND verifier IS NULL";
