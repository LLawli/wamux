//! #164: encryption at rest for the key material in the store. The tests start
//! from the claim a leaked dump is not a takeover, so they look at what is
//! stored, not only at what the traits return: every sealed column starts with
//! the blob header, and the database file holds none of the known secrets.
//!
//! SQLite and Turso need no server. Postgres gets a throwaway database (the
//! `wamux_upgrade_` prefix `account_leftovers` sweeps), because the shared
//! `wamux` database would turn `encrypted` for good the first time a keyed
//! test touched it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use sqlx::PgPool;
use wacore::appstate::hash::HashState;
use wacore::appstate::processor::AppStateMutationMAC;
use wacore::store::traits::{AppStateSyncKey, Backend, MsgSecretEntry, TcTokenEntry};
use wamux::storage::sql::{SqlPool, SqlStore};
use wamux::storage::{StorageEngine, StoreKey};

#[allow(dead_code)]
mod common;

const ALICE: &str = "alice@s.whatsapp.net";
const BOB: &str = "bob@s.whatsapp.net";
const CHAT: &str = "5511900000001@s.whatsapp.net";
const ME: &str = "5511900000000@s.whatsapp.net";
const TC_JID: &str = "100000000000001@lid";

/// A secret long enough that finding it by accident is not a thing.
const MARKER: &[u8] = b"SECRET-MARKER-0123456789-abcdefghij";
const KEY_A: &str = "ab";
const KEY_B: &str = "cd";

fn key(byte: &str) -> StoreKey {
    StoreKey::parse_hex(&byte.repeat(32)).expect("a valid test key")
}

/// Every sealed column, as `(table, column)`.
const SEALED: [(&str, &str); 13] = [
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
trait Raw: Send + Sync {
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
        let rows = common::turso_rows(self, &sql, Vec::new()).await;
        rows.into_iter()
            .map(|mut row| match row.remove(0) {
                turso::Value::Blob(bytes) => bytes,
                other => panic!("{table}.{column} is not a blob: {other:?}"),
            })
            .collect()
    }

    async fn encryption_state(&self) -> String {
        let sql = "SELECT state FROM store_encryption WHERE id = 1";
        let rows = common::turso_rows(self, sql, Vec::new()).await;
        match &rows[0][0] {
            turso::Value::Text(state) => state.clone(),
            other => panic!("store_encryption.state is not text: {other:?}"),
        }
    }
}

/// Write one secret into each of the 13 sealed columns, through the traits.
async fn write_every_secret(b: &dyn Backend) {
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
async fn assert_every_secret_reads_back(b: &dyn Backend) {
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
async fn assert_every_column_is_sealed(raw: &dyn Raw, key: &StoreKey) {
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
fn dump_of(db: &Path) -> Vec<u8> {
    let mut dump = std::fs::read(db).unwrap();
    let mut wal = db.as_os_str().to_owned();
    wal.push("-wal");
    dump.extend(std::fs::read(PathBuf::from(wal)).unwrap_or_default());
    dump
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn assert_dump_hides_the_secrets(dump: &[u8]) {
    assert!(!contains(dump, MARKER), "the marker is in the dump");
    for byte in [0xa1u8, 0xd0, 0x5c, 0x7a] {
        assert!(
            !contains(dump, &[byte; 16]),
            "a run of {byte:#x} (a known key) is in the dump"
        );
    }
}

fn sqlite_url(dir: &Path) -> String {
    format!("sqlite://{}", dir.join("wamux.db").display())
}

#[tokio::test]
async fn sqlite_encrypted_store_round_trips_every_sealed_table_and_hides_the_keys() {
    let dir = tempfile::tempdir().unwrap();
    let key = key(KEY_A);
    let store = SqlStore::open_sqlite_keyed(&sqlite_url(dir.path()), Some(&key))
        .await
        .expect("open a new store with a key");
    let account = store.create_account(Some("enc/a")).await.unwrap();
    let backend = store.device_backend(account.device_id);
    write_every_secret(&*backend).await;
    assert_every_secret_reads_back(&*backend).await;
    write_every_secret(&*backend).await;
    assert_every_column_is_sealed(&store, &key).await;
    assert_dump_hides_the_secrets(&dump_of(&dir.path().join("wamux.db")));
}

#[tokio::test]
async fn sqlite_a_new_keyed_store_is_marked_encrypted_and_reopens_with_the_same_key() {
    let dir = tempfile::tempdir().unwrap();
    let url = sqlite_url(dir.path());
    let key = key(KEY_A);
    let store = SqlStore::open_sqlite_keyed(&url, Some(&key)).await.unwrap();
    assert_eq!(store.encryption_state().await, "encrypted");
    let account = store.create_account(Some("enc/a")).await.unwrap();
    store
        .device_backend(account.device_id)
        .put_session(ALICE, MARKER)
        .await
        .unwrap();
    common::lite_pool(&store).close().await;

    let again = SqlStore::open_sqlite_keyed(&url, Some(&key)).await.unwrap();
    let backend = again.device_backend(account.device_id);
    assert_eq!(
        backend.get_session(ALICE).await.unwrap(),
        Some(Bytes::from_static(MARKER)),
        "the same key opens what it sealed"
    );
}

#[tokio::test]
async fn sqlite_wrong_key_refuses_to_open_with_does_not_match() {
    let dir = tempfile::tempdir().unwrap();
    let url = sqlite_url(dir.path());
    let store = SqlStore::open_sqlite_keyed(&url, Some(&key(KEY_A)))
        .await
        .unwrap();
    common::lite_pool(&store).close().await;
    let error = SqlStore::open_sqlite_keyed(&url, Some(&key(KEY_B)))
        .await
        .err()
        .expect("a different key must be refused at open");
    assert!(error.to_string().contains("does not match"), "{error}");
}

#[tokio::test]
async fn sqlite_encrypted_store_without_a_key_refuses_to_open() {
    let dir = tempfile::tempdir().unwrap();
    let url = sqlite_url(dir.path());
    let store = SqlStore::open_sqlite_keyed(&url, Some(&key(KEY_A)))
        .await
        .unwrap();
    common::lite_pool(&store).close().await;
    let error = SqlStore::open_sqlite(&url)
        .await
        .err()
        .expect("an encrypted store needs its key");
    let shown = error.to_string();
    assert!(shown.contains("store_key_file"), "{shown}");
    assert!(shown.contains("encrypted"), "{shown}");
}

#[tokio::test]
async fn sqlite_plaintext_store_with_accounts_refuses_a_key_until_converted() {
    let dir = tempfile::tempdir().unwrap();
    let url = sqlite_url(dir.path());
    let store = SqlStore::open_sqlite(&url).await.unwrap();
    store.create_account(Some("enc/plain")).await.unwrap();
    common::lite_pool(&store).close().await;
    let error = SqlStore::open_sqlite_keyed(&url, Some(&key(KEY_A)))
        .await
        .err()
        .expect("accounts are plaintext: converting them is #165");
    let shown = error.to_string();
    assert!(shown.contains("not encrypted yet"), "{shown}");
    assert!(shown.contains("convert"), "{shown}");
}

#[tokio::test]
async fn sqlite_plaintext_store_without_a_key_still_works_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqlStore::open_sqlite(&sqlite_url(dir.path()))
        .await
        .unwrap();
    assert_eq!(store.encryption_state().await, "plaintext");
    let account = store.create_account(Some("enc/plain")).await.unwrap();
    let backend = store.device_backend(account.device_id);
    backend.put_session(ALICE, MARKER).await.unwrap();
    assert_eq!(
        store.blobs("sessions", "record").await,
        vec![MARKER.to_vec()],
        "no key: the bytes are stored as they always were"
    );
}

#[tokio::test]
async fn sqlite_a_blob_moved_to_another_row_or_device_fails_to_open() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqlStore::open_sqlite_keyed(&sqlite_url(dir.path()), Some(&key(KEY_A)))
        .await
        .unwrap();
    let a = store.create_account(Some("enc/a")).await.unwrap();
    let b = store.create_account(Some("enc/b")).await.unwrap();
    let (ba, bb) = (
        store.device_backend(a.device_id),
        store.device_backend(b.device_id),
    );
    ba.put_session(ALICE, b"alice's").await.unwrap();
    ba.put_session(BOB, b"bob's").await.unwrap();
    bb.put_session(ALICE, b"b's own").await.unwrap();
    let pool = common::lite_pool(&store);

    // Alice's blob copied over Bob's row, same account: another row.
    sqlx::query(
        "UPDATE sessions SET record = (SELECT record FROM sessions WHERE address = $1 AND device_id = $2) \
         WHERE address = $3 AND device_id = $2",
    )
    .bind(ALICE)
    .bind(a.device_id)
    .bind(BOB)
    .execute(pool)
    .await
    .unwrap();
    assert!(ba.get_session(BOB).await.is_err(), "another row opened");
    assert!(
        ba.get_session(ALICE).await.is_ok(),
        "the home row still opens"
    );

    // Account a's Alice blob copied over account b's Alice row: another device.
    sqlx::query(
        "UPDATE sessions SET record = (SELECT record FROM sessions WHERE address = $1 AND device_id = $2) \
         WHERE address = $1 AND device_id = $3",
    )
    .bind(ALICE)
    .bind(a.device_id)
    .bind(b.device_id)
    .execute(pool)
    .await
    .unwrap();
    assert!(
        bb.get_session(ALICE).await.is_err(),
        "another device opened"
    );
}

#[tokio::test]
async fn sqlite_an_empty_tc_token_stays_empty_in_the_clear() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqlStore::open_sqlite_keyed(&sqlite_url(dir.path()), Some(&key(KEY_A)))
        .await
        .unwrap();
    let account = store.create_account(Some("enc/a")).await.unwrap();
    let backend = store.device_backend(account.device_id);
    let empty = TcTokenEntry {
        token: Vec::new(),
        token_timestamp: 1_749_400_000,
        sender_timestamp: Some(1_749_400_100),
    };
    backend.put_tc_token(TC_JID, &empty).await.unwrap();
    let length: i64 = sqlx::query_scalar("SELECT length(token) FROM tc_tokens WHERE jid = $1")
        .bind(TC_JID)
        .fetch_one(common::lite_pool(&store))
        .await
        .unwrap();
    assert_eq!(length, 0, "length(token) = 0 is what the SQL tests for");
    let read = backend.get_tc_token(TC_JID).await.unwrap().expect("row");
    assert!(read.token.is_empty());
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_encrypted_store_round_trips_and_hides_the_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wamux.db");
    let key = key(KEY_A);
    let url = format!("turso://{}", path.display());
    let store = wamux::storage::turso::TursoStore::open_keyed(&url, Some(&key))
        .await
        .expect("open a new turso store with a key");
    let account = store.create_account(Some("enc/a")).await.unwrap();
    let backend = store.device_backend(account.device_id);
    write_every_secret(&*backend).await;
    assert_every_secret_reads_back(&*backend).await;
    write_every_secret(&*backend).await;
    assert_every_column_is_sealed(&store, &key).await;
    assert_eq!(store.encryption_state().await, "encrypted");
    // The backend holds a handle on the one connection; `close` needs it gone.
    drop(backend);
    store.close().await.expect("release the file");
    assert_dump_hides_the_secrets(&dump_of(&path));
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_wrong_key_refuses_to_open_with_does_not_match() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("turso://{}", dir.path().join("wamux.db").display());
    let store = wamux::storage::turso::TursoStore::open_keyed(&url, Some(&key(KEY_A)))
        .await
        .unwrap();
    store.close().await.unwrap();
    let error = wamux::storage::turso::TursoStore::open_keyed(&url, Some(&key(KEY_B)))
        .await
        .err()
        .expect("a different key must be refused at open");
    assert!(error.to_string().contains("does not match"), "{error}");
}

/// A database of its own, dropped by the test. The `wamux_upgrade_` prefix is
/// the one `account_leftovers` sweeps if a run is killed before the drop.
async fn throwaway_postgres() -> (String, String) {
    let name = format!("wamux_upgrade_{}", uuid::Uuid::new_v4().simple());
    let admin = PgPool::connect(&common::database_url()).await.unwrap();
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    let base = common::database_url();
    let url = format!("{}/{name}", base.rsplit_once('/').unwrap().0);
    (url, name)
}

#[tokio::test]
async fn postgres_encrypted_store_round_trips_and_hides_the_keys() {
    let (url, name) = throwaway_postgres().await;
    let key = key(KEY_A);
    let store = SqlStore::open_postgres_keyed(&url, 2, Some(&key))
        .await
        .expect("open a new postgres store with a key");
    let account = store.create_account(Some("enc/a")).await.unwrap();
    let backend = store.device_backend(account.device_id);
    write_every_secret(&*backend).await;
    assert_every_secret_reads_back(&*backend).await;
    write_every_secret(&*backend).await;
    assert_every_column_is_sealed(&store, &key).await;
    assert_eq!(store.encryption_state().await, "encrypted");
    common::pg_pool(&store).close().await;

    let admin = PgPool::connect(&common::database_url()).await.unwrap();
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
