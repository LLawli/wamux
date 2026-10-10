//! What the encryption tests share (#164, #165): the secrets written into every
//! sealed column, the checks that they read back and are sealed on disk, the raw
//! reads below the store traits, and a throwaway Postgres database.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use sqlx::PgPool;
use wacore::appstate::hash::HashState;
use wacore::appstate::processor::AppStateMutationMAC;
use wacore::store::traits::{AppStateSyncKey, Backend, MsgSecretEntry, TcTokenEntry};
use wamux::storage::StoreKey;
use wamux::storage::sql::{SqlPool, SqlStore};

pub const ALICE: &str = "alice@s.whatsapp.net";
pub const BOB: &str = "bob@s.whatsapp.net";
pub const CHAT: &str = "5511900000001@s.whatsapp.net";
pub const ME: &str = "5511900000000@s.whatsapp.net";
pub const TC_JID: &str = "100000000000001@lid";

/// A secret long enough that finding it by accident is not a thing.
pub const MARKER: &[u8] = b"SECRET-MARKER-0123456789-abcdefghij";
pub const KEY_A: &str = "ab";
pub const KEY_B: &str = "cd";

pub fn key(byte: &str) -> StoreKey {
    StoreKey::parse_hex(&byte.repeat(32)).expect("a valid test key")
}

/// Every sealed column, as `(table, column)`.
pub const SEALED: [(&str, &str); 13] = [
    ("device", "data"),
    ("identities", "key"),
    ("sessions", "record"),
    ("prekeys", "key"),
    ("signed_prekeys", "record"),
    ("sender_keys", "record"),
    ("app_state_keys", "key_data"),
    ("app_state_versions", "state_data"),
    ("app_state_mutation_macs", "value_mac"),
    ("base_keys", "base_key"),
    ("tc_tokens", "token"),
    ("msg_secrets", "secret"),
    ("sent_messages", "payload"),
];

/// Reads below the store traits, one per engine.
#[async_trait]
pub trait Raw: Send + Sync {
    /// Every non-empty value of a BLOB column, across all accounts.
    async fn blobs(&self, table: &str, column: &str) -> Vec<Vec<u8>>;
    /// One TEXT value from the single-row `store_encryption` table.
    async fn encryption_state(&self) -> String;
}

#[async_trait]
impl Raw for SqlStore {
    async fn blobs(&self, table: &str, column: &str) -> Vec<Vec<u8>> {
        let sql = format!("SELECT {column} FROM {table} WHERE length({column}) > 0");
        match self.pool() {
            SqlPool::Pg(pool) => sqlx::query_scalar(&sql).fetch_all(pool).await,
            SqlPool::Sqlite(pool) => sqlx::query_scalar(&sql).fetch_all(pool).await,
        }
        .unwrap_or_else(|e| panic!("read {table}.{column}: {e}"))
    }

    async fn encryption_state(&self) -> String {
        let sql = "SELECT state FROM store_encryption WHERE id = 1";
        match self.pool() {
            SqlPool::Pg(pool) => sqlx::query_scalar(sql).fetch_one(pool).await,
            SqlPool::Sqlite(pool) => sqlx::query_scalar(sql).fetch_one(pool).await,
        }
        .expect("read store_encryption.state")
    }
}

#[cfg(feature = "turso")]
#[async_trait]
impl Raw for wamux::storage::turso::TursoStore {
    async fn blobs(&self, table: &str, column: &str) -> Vec<Vec<u8>> {
        let sql = format!("SELECT {column} FROM {table} WHERE length({column}) > 0");
        let rows = super::turso_rows(self, &sql, Vec::new()).await;
        rows.into_iter()
            .map(|mut row| match row.remove(0) {
                turso::Value::Blob(bytes) => bytes,
                other => panic!("{table}.{column} is not a blob: {other:?}"),
            })
            .collect()
    }

    async fn encryption_state(&self) -> String {
        let sql = "SELECT state FROM store_encryption WHERE id = 1";
        let rows = super::turso_rows(self, sql, Vec::new()).await;
        match &rows[0][0] {
            turso::Value::Text(state) => state.clone(),
            other => panic!("store_encryption.state is not text: {other:?}"),
        }
    }
}

/// Write one secret into each of the 13 sealed columns, through the traits.
pub async fn write_every_secret(b: &dyn Backend) {
    b.create().await.expect("create the device row");
    b.put_identity(ALICE, [0xa1; 32]).await.unwrap();
    b.put_session(ALICE, MARKER).await.unwrap();
    b.store_prekey(7, MARKER, true).await.unwrap();
    b.store_signed_prekey(3, MARKER).await.unwrap();
    b.put_sender_key("120363000000000001@g.us::alice", MARKER)
        .await
        .unwrap();
    let sync_key = AppStateSyncKey {
        key_data: vec![0xd0; 32],
        fingerprint: vec![1, 2, 3],
        timestamp: 1_749_400_000,
    };
    b.set_sync_key(&[0x00, 0x01], sync_key).await.unwrap();
    let state = HashState {
        version: 42,
        hash: [0x5a; 128],
        ..HashState::default()
    };
    b.set_version("regular_low", state).await.unwrap();
    let macs = [AppStateMutationMAC {
        index_mac: vec![0x1d; 32],
        value_mac: vec![0x7a; 32],
    }];
    b.put_mutation_macs("regular_low", 42, &macs).await.unwrap();
    b.save_base_key("alice.0", "M1", MARKER).await.unwrap();
    let token = TcTokenEntry {
        token: MARKER.to_vec(),
        token_timestamp: 1_749_400_000,
        sender_timestamp: Some(1_749_400_100),
    };
    b.put_tc_token(TC_JID, &token).await.unwrap();
    b.store_sent_message(CHAT, "M1", MARKER).await.unwrap();
    let secret = MsgSecretEntry {
        chat: Arc::from(CHAT),
        sender: Arc::from(ME),
        msg_id: Arc::from("M1"),
        secret: [0x5c; 32],
        expires_at: 0,
        message_ts: 1_749_400_000,
    };
    b.put_msg_secrets(vec![secret]).await.unwrap();
}

/// Everything `write_every_secret` wrote comes back unchanged.
pub async fn assert_every_secret_reads_back(b: &dyn Backend) {
    assert!(b.load().await.unwrap().is_some(), "device");
    assert_eq!(b.load_identity(ALICE).await.unwrap(), Some([0xa1; 32]));
    assert_eq!(
        b.get_session(ALICE).await.unwrap(),
        Some(Bytes::from_static(MARKER))
    );
    assert_eq!(
        b.load_prekey(7).await.unwrap(),
        Some(Bytes::from_static(MARKER))
    );
    assert_eq!(
        b.load_signed_prekey(3).await.unwrap(),
        Some(MARKER.to_vec())
    );
    assert_eq!(
        b.get_sender_key("120363000000000001@g.us::alice")
            .await
            .unwrap(),
        Some(MARKER.to_vec())
    );
    let sync_key = b.get_sync_key(&[0x00, 0x01]).await.unwrap().expect("key");
    assert_eq!(sync_key.key_data, vec![0xd0; 32]);
    let state = b.get_version("regular_low").await.unwrap().expect("state");
    assert_eq!(state.version, 42);
    assert_eq!(
        b.get_mutation_mac("regular_low", &[0x1d; 32])
            .await
            .unwrap(),
        Some(vec![0x7a; 32])
    );
    assert!(b.has_same_base_key("alice.0", "M1", MARKER).await.unwrap());
    let token = b.get_tc_token(TC_JID).await.unwrap().expect("tc token");
    assert_eq!(token.token, MARKER);
    assert_eq!(
        b.take_sent_message(CHAT, "M1").await.unwrap(),
        Some(MARKER.to_vec())
    );
    assert_eq!(
        b.get_msg_secret(CHAT, ME, "M1").await.unwrap(),
        Some(vec![0x5c; 32])
    );
}

/// Every non-empty value of every sealed column starts with the blob header of
/// `key`, and there is at least one value in each column.
pub async fn assert_every_column_is_sealed(raw: &dyn Raw, key: &StoreKey) {
    for (table, column) in SEALED {
        let blobs = raw.blobs(table, column).await;
        assert!(!blobs.is_empty(), "{table}.{column} has no rows to inspect");
        for blob in blobs {
            assert_eq!(blob[0], 1, "{table}.{column}: blob version");
            assert_eq!(blob[1..5], key.id(), "{table}.{column}: key id");
        }
    }
}

/// The database file and its WAL, as bytes: what a copy of the store holds.
pub fn dump_of(db: &Path) -> Vec<u8> {
    let mut dump = std::fs::read(db).unwrap();
    let mut wal = db.as_os_str().to_owned();
    wal.push("-wal");
    dump.extend(std::fs::read(PathBuf::from(wal)).unwrap_or_default());
    dump
}

pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

pub fn assert_dump_hides_the_secrets(dump: &[u8]) {
    assert!(!contains(dump, MARKER), "the marker is in the dump");
    for byte in [0xa1u8, 0xd0, 0x5c, 0x7a] {
        assert!(
            !contains(dump, &[byte; 16]),
            "a run of {byte:#x} (a known key) is in the dump"
        );
    }
}

pub fn sqlite_url(dir: &Path) -> String {
    format!("sqlite://{}", dir.join("wamux.db").display())
}

/// A database of its own, dropped by the test. The `wamux_upgrade_` prefix is
/// the one `account_leftovers` sweeps if a run is killed before the drop.
pub async fn throwaway_postgres() -> (String, String) {
    let name = format!("wamux_upgrade_{}", uuid::Uuid::new_v4().simple());
    let admin = PgPool::connect(&super::database_url()).await.unwrap();
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    let base = super::database_url();
    let url = format!("{}/{name}", base.rsplit_once('/').unwrap().0);
    (url, name)
}

/// Drop a database `throwaway_postgres` made. The pool of the store opened on it
/// must be closed first.
pub async fn drop_throwaway_postgres(name: &str) {
    let admin = PgPool::connect(&super::database_url()).await.unwrap();
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
