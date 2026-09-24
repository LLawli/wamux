//! The bincode -> protobuf conversion end to end (#31), on both engines: a store
//! as a pre-#31 daemon left it (SQL migrations applied, blobs in bincode, marker
//! at 'bincode') is opened the way the daemon opens it, and must come out
//! converted and loadable, every field carried over as it was (`bootstrapped`
//! included: since #36 the library marks a collection at the head itself).
//!
//! The Postgres cases run in a throwaway database of their own: the conversion
//! is store-wide, so the shared test database cannot hold a legacy store.

// Only a subset of the shared helpers is used per test binary.
#[allow(dead_code)]
mod common;

use std::collections::HashMap;
use std::sync::Arc;

use sqlx::{PgPool, SqlitePool};
use wacore::appstate::hash::HashState;
use wacore::store::Device;
use wacore::store::traits::AppStateSyncKey;
use wamux::storage::StorageEngine;
use wamux::storage::postgres::{self, PgStorage};
use wamux::storage::sqlite::{self, SqliteStorage};
use whatsapp_rust::Jid;

const KEY_ID: &[u8] = &[0, 0, 0, 1];

fn bincode_of<T: serde::Serialize>(value: &T) -> Vec<u8> {
    bincode::serde::encode_to_vec(value, bincode::config::standard()).unwrap()
}

/// What a pre-#31 daemon had written for one account.
struct LegacyAccount {
    device: Device,
    /// `(collection, state)`: one synced but unmarked, one marked. Both must
    /// come out exactly as they went in.
    versions: Vec<(&'static str, HashState)>,
    sync_key: AppStateSyncKey,
}

fn hash_state(version: u64, bootstrapped: bool, mac_mismatch_fatal: bool) -> HashState {
    HashState {
        version,
        hash: [0x2D; 128],
        index_value_map: HashMap::from([("idx".to_string(), vec![9u8, 8])]),
        mac_mismatch_fatal,
        bootstrapped,
    }
}

fn legacy_account() -> LegacyAccount {
    let mut device = Device::new();
    // unwrap: parsing literal, well-formed JIDs.
    device.pn = Some("559980000001@s.whatsapp.net".parse::<Jid>().unwrap());
    device.lid = Some("169815004184633@lid".parse::<Jid>().unwrap());
    device.push_name = "upgrade test".to_string();
    device.next_pre_key_id = 812;
    device.lid_migrated = true;
    LegacyAccount {
        device,
        versions: vec![
            ("regular_low", hash_state(1399, false, true)),
            ("critical_unblock_low", hash_state(12, true, false)),
        ],
        sync_key: AppStateSyncKey {
            key_data: vec![0x42; 32],
            fingerprint: vec![1, 2, 3],
            timestamp: 1_749_400_000,
        },
    }
}

/// Every field bincode persists, which is every field the store keeps.
fn persisted(device: &Device) -> Vec<u8> {
    bincode_of(device)
}

async fn assert_converted(engine: Arc<dyn StorageEngine>, device_id: i32, legacy: &LegacyAccount) {
    let backend = engine.device_backend(device_id);
    let device = backend
        .load()
        .await
        .unwrap()
        .expect("device loads after conversion");
    assert!(
        persisted(&device) == persisted(&legacy.device),
        "every persisted Device field, keys included, must survive the conversion"
    );

    let unmarked = backend.get_version("regular_low").await.unwrap().unwrap();
    assert!(
        !unmarked.bootstrapped,
        "synced but unmarked stays unmarked: the first sync at the head marks it (#36)"
    );
    assert!(
        unmarked.mac_mismatch_fatal,
        "the latch is carried over untouched"
    );
    assert_eq!(unmarked.version, 1399);
    let untouched = backend
        .get_version("critical_unblock_low")
        .await
        .unwrap()
        .unwrap();
    assert!(untouched.bootstrapped);
    assert_eq!(untouched.version, 12);

    let key = backend
        .get_sync_key(KEY_ID)
        .await
        .unwrap()
        .expect("sync key");
    assert_eq!(key.key_data, legacy.sync_key.key_data);
    assert_eq!(key.fingerprint, legacy.sync_key.fingerprint);
    assert_eq!(key.timestamp, legacy.sync_key.timestamp);
}

// --- SQLite ---

/// A legacy SQLite store: SQL migrations only (so the marker reads 'bincode'),
/// one account, its blobs written as bincode. `garbage_device` adds a second
/// account whose device blob is not bincode at all.
async fn legacy_sqlite(
    legacy: &LegacyAccount,
    garbage_device: bool,
) -> (String, tempfile::TempDir, i32) {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("legacy.db").display()
    );
    let pool = sqlite::connect(&url).await.unwrap();
    sqlite::MIGRATOR.run(&pool).await.unwrap();
    let accounts = SqliteStorage::from_pool(pool.clone());
    let device_id = accounts
        .create_account(Some("upgrade"))
        .await
        .unwrap()
        .device_id;
    seed_sqlite(&pool, device_id, legacy).await;
    if garbage_device {
        let other = accounts
            .create_account(Some("garbage"))
            .await
            .unwrap()
            .device_id;
        insert_device_sqlite(&pool, other, &[0xFF; 9]).await;
    }
    pool.close().await;
    (url, dir, device_id)
}

async fn insert_device_sqlite(pool: &SqlitePool, device_id: i32, blob: &[u8]) {
    sqlx::query("INSERT INTO device (device_id, data) VALUES (?, ?)")
        .bind(device_id)
        .bind(blob)
        .execute(pool)
        .await
        .unwrap();
}

async fn seed_sqlite(pool: &SqlitePool, device_id: i32, legacy: &LegacyAccount) {
    insert_device_sqlite(pool, device_id, &bincode_of(&legacy.device)).await;
    for (name, state) in &legacy.versions {
        sqlx::query(
            "INSERT INTO app_state_versions (name, state_data, device_id) VALUES (?, ?, ?)",
        )
        .bind(name)
        .bind(bincode_of(state))
        .bind(device_id)
        .execute(pool)
        .await
        .unwrap();
    }
    sqlx::query("INSERT INTO app_state_keys (key_id, key_data, device_id) VALUES (?, ?, ?)")
        .bind(KEY_ID)
        .bind(bincode_of(&legacy.sync_key))
        .bind(device_id)
        .execute(pool)
        .await
        .unwrap();
}

async fn sqlite_marker(pool: &SqlitePool) -> String {
    sqlx::query_scalar("SELECT format FROM blob_format WHERE id = 1")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn sqlite_device_blob(pool: &SqlitePool, device_id: i32) -> Vec<u8> {
    sqlx::query_scalar("SELECT data FROM device WHERE device_id = ?")
        .bind(device_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn sqlite_legacy_store_is_converted_on_open() {
    let legacy = legacy_account();
    let (url, _dir, device_id) = legacy_sqlite(&legacy, false).await;

    let engine = SqliteStorage::open(&url).await.expect("open converts");
    assert_eq!(sqlite_marker(engine.pool()).await, "protobuf");
    let converted = sqlite_device_blob(engine.pool(), device_id).await;
    assert_converted(Arc::new(engine), device_id, &legacy).await;

    // Idempotent: a second open finds 'protobuf' and writes nothing.
    let again = SqliteStorage::open(&url).await.expect("reopen");
    assert_eq!(sqlite_device_blob(again.pool(), device_id).await, converted);
}

#[tokio::test]
async fn sqlite_one_unreadable_row_aborts_the_conversion_and_changes_nothing() {
    let legacy = legacy_account();
    let (url, _dir, device_id) = legacy_sqlite(&legacy, true).await;

    let err = SqliteStorage::open(&url)
        .await
        .err()
        .expect("open must refuse");
    let chain = format!("{:?}", anyhow::Error::from(err));
    assert!(chain.contains("not readable as bincode"), "{chain}");

    let pool = sqlite::connect(&url).await.unwrap();
    assert_eq!(
        sqlite_marker(&pool).await,
        "bincode",
        "the marker did not move"
    );
    assert_eq!(
        sqlite_device_blob(&pool, device_id).await,
        bincode_of(&legacy.device),
        "the good row is still the bincode it was"
    );
}

#[tokio::test]
async fn sqlite_fresh_store_is_marked_protobuf() {
    let (engine, _dir) = common::sqlite_engine().await;
    assert_eq!(sqlite_marker(engine.pool()).await, "protobuf");
}

// --- Postgres ---

/// A database of its own, so the store-wide conversion touches nothing shared.
/// Dropped by `drop_pg_database` at the end of the test.
struct ThrowawayPg {
    name: String,
    url: String,
}

async fn create_pg_database() -> ThrowawayPg {
    let admin = PgPool::connect(&common::database_url()).await.unwrap();
    let name = format!("wamux_upgrade_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    let base = common::database_url();
    let url = format!("{}/{name}", base.rsplit_once('/').unwrap().0);
    ThrowawayPg { name, url }
}

async fn drop_pg_database(db: ThrowawayPg) {
    let admin = PgPool::connect(&common::database_url()).await.unwrap();
    sqlx::query(&format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", db.name))
        .execute(&admin)
        .await
        .unwrap();
}

async fn legacy_pg(legacy: &LegacyAccount) -> (ThrowawayPg, i32) {
    let db = create_pg_database().await;
    let pool = postgres::connect(&db.url, 2).await.unwrap();
    postgres::MIGRATOR.run(&pool).await.unwrap();
    let accounts = PgStorage::from_pool(pool.clone());
    let device_id = accounts
        .create_account(Some("upgrade"))
        .await
        .unwrap()
        .device_id;
    seed_pg(&pool, device_id, legacy).await;
    pool.close().await;
    (db, device_id)
}

async fn seed_pg(pool: &PgPool, device_id: i32, legacy: &LegacyAccount) {
    sqlx::query("INSERT INTO device (device_id, data) VALUES ($1, $2)")
        .bind(device_id)
        .bind(bincode_of(&legacy.device))
        .execute(pool)
        .await
        .unwrap();
    for (name, state) in &legacy.versions {
        sqlx::query(
            "INSERT INTO app_state_versions (name, state_data, device_id) VALUES ($1, $2, $3)",
        )
        .bind(name)
        .bind(bincode_of(state))
        .bind(device_id)
        .execute(pool)
        .await
        .unwrap();
    }
    sqlx::query("INSERT INTO app_state_keys (key_id, key_data, device_id) VALUES ($1, $2, $3)")
        .bind(KEY_ID)
        .bind(bincode_of(&legacy.sync_key))
        .bind(device_id)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn postgres_legacy_store_is_converted_on_open() {
    let legacy = legacy_account();
    let (db, device_id) = legacy_pg(&legacy).await;

    let engine = PgStorage::open(&db.url, 2).await.expect("open converts");
    let marker: String = sqlx::query_scalar("SELECT format FROM blob_format WHERE id = 1")
        .fetch_one(engine.pool())
        .await
        .unwrap();
    assert_eq!(marker, "protobuf");
    let pool = engine.pool().clone();
    assert_converted(Arc::new(engine), device_id, &legacy).await;

    pool.close().await;
    drop_pg_database(db).await;
}

// --- Rehearsal on a copy of a real store ---

/// Every row of a real legacy store, decoded as bincode before the conversion.
struct RealStoreBefore {
    devices: Vec<(i32, Device)>,
    versions: Vec<(i32, String, HashState)>,
    sync_keys: Vec<(i32, Vec<u8>, AppStateSyncKey)>,
}

fn from_bincode<T: serde::de::DeserializeOwned>(blob: &[u8]) -> T {
    bincode::serde::decode_from_slice(blob, bincode::config::standard())
        .expect("the copy must still be bincode: rehearse on a fresh copy")
        .0
}

async fn read_real_store(pool: &SqlitePool) -> RealStoreBefore {
    let devices: Vec<(i32, Vec<u8>)> = sqlx::query_as("SELECT device_id, data FROM device")
        .fetch_all(pool)
        .await
        .unwrap();
    let versions: Vec<(i32, String, Vec<u8>)> =
        sqlx::query_as("SELECT device_id, name, state_data FROM app_state_versions")
            .fetch_all(pool)
            .await
            .unwrap();
    let keys: Vec<(i32, Vec<u8>, Vec<u8>)> =
        sqlx::query_as("SELECT device_id, key_id, key_data FROM app_state_keys")
            .fetch_all(pool)
            .await
            .unwrap();
    RealStoreBefore {
        devices: devices
            .into_iter()
            .map(|(d, b)| (d, from_bincode(&b)))
            .collect(),
        versions: versions
            .into_iter()
            .map(|(d, n, b)| (d, n, from_bincode(&b)))
            .collect(),
        sync_keys: keys
            .into_iter()
            .map(|(d, k, b)| (d, k, from_bincode(&b)))
            .collect(),
    }
}

/// A live daemon keeps its socket next to the store; a copy has none. The
/// rehearsal converts what it opens, so it refuses anything that looks live.
fn refuse_a_live_store(url: &str) {
    let path = url
        .trim_start_matches("sqlite://")
        .split('?')
        .next()
        .unwrap();
    let dir = std::path::Path::new(path).parent().unwrap();
    assert!(
        !dir.join("wamux.sock").exists(),
        "{path} sits next to a wamux.sock: rehearse on a COPY (sqlite3 .backup), never the live store"
    );
}

/// Every field carried over as it was, `bootstrapped` included (#36). Returns
/// how many rows are still unmarked with a baseline: the library's first sync
/// at the head marks each of them. Counts only on stdout: the store holds
/// Signal key material.
async fn check_versions(engine: &SqliteStorage, before: &RealStoreBefore) -> usize {
    let mut unmarked = 0;
    for (device_id, name, old) in &before.versions {
        let new = engine
            .device_backend(*device_id)
            .get_version(name)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(new.version, old.version, "{name}");
        assert_eq!(new.hash, old.hash, "{name}");
        assert_eq!(new.index_value_map, old.index_value_map, "{name}");
        assert_eq!(new.mac_mismatch_fatal, old.mac_mismatch_fatal, "{name}");
        assert_eq!(new.bootstrapped, old.bootstrapped, "{name}");
        if !new.bootstrapped && new.version > 0 {
            unmarked += 1;
        }
    }
    unmarked
}

/// `WAMUX_REHEARSAL_DB=sqlite:///path/to/copy.db cargo test --test bincode_upgrade
/// rehearse -- --ignored --nocapture`. Converts the copy exactly as the daemon
/// would on its first start, then proves every row against what it held before.
#[tokio::test]
#[ignore = "needs WAMUX_REHEARSAL_DB pointing at a copy of a real store"]
async fn rehearse_the_conversion_on_a_copy_of_a_real_store() {
    let url = std::env::var("WAMUX_REHEARSAL_DB").expect("set WAMUX_REHEARSAL_DB");
    refuse_a_live_store(&url);
    let pool = sqlite::connect(&url).await.unwrap();
    let before = read_real_store(&pool).await;
    pool.close().await;

    let engine = SqliteStorage::open(&url).await.expect("open converts");
    assert_eq!(sqlite_marker(engine.pool()).await, "protobuf");
    for (device_id, old) in &before.devices {
        let new = engine
            .device_backend(*device_id)
            .load()
            .await
            .unwrap()
            .unwrap();
        assert!(persisted(&new) == persisted(old), "device {device_id}");
    }
    for (device_id, key_id, old) in &before.sync_keys {
        let new = engine
            .device_backend(*device_id)
            .get_sync_key(key_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (&new.key_data, &new.fingerprint, new.timestamp),
            (&old.key_data, &old.fingerprint, old.timestamp)
        );
    }
    let unmarked = check_versions(&engine, &before).await;
    println!(
        "rehearsal ok: {} devices, {} app-state versions, {} sync keys; {unmarked} synced but unmarked, left to the first sync",
        before.devices.len(),
        before.versions.len(),
        before.sync_keys.len()
    );
}
