//! A store as a pre-#31 daemon left it: SQL migrations applied, blobs in
//! bincode, marker at 'bincode'. Shared by `bincode_upgrade` (the conversion)
//! and `bincode_upgrade_notice` (what the operator is told about it, #84).

use std::collections::HashMap;

use sqlx::SqlitePool;
use wacore::appstate::hash::HashState;
use wacore::store::Device;
use wacore::store::traits::AppStateSyncKey;
use wamux::storage::StorageEngine;
use wamux::storage::blob_codec::decode_device;
use wamux::storage::sql::{self, SqlPool, SqlStore};

pub const KEY_ID: &[u8] = &[0, 0, 0, 1];

/// A real device blob written by the 23846f7e build, and the protobuf of the
/// same value (#86). Bincode is positional: a blob built here from the current
/// `Device` would only prove the current layout converts, never the layout a
/// legacy store actually holds. See tests/fixtures/bincode-23846f7e/README.md.
pub const DEVICE_BINCODE: &[u8] = include_bytes!("../fixtures/bincode-23846f7e/device.bincode");
pub const DEVICE_PB: &[u8] = include_bytes!("../fixtures/bincode-23846f7e/device.pb");

pub fn bincode_of<T: serde::Serialize>(value: &T) -> Vec<u8> {
    bincode::serde::encode_to_vec(value, bincode::config::standard()).unwrap()
}

/// What a pre-#31 daemon had written for one account.
pub struct LegacyAccount {
    /// The device row as the legacy store holds it.
    pub device_bincode: &'static [u8],
    /// What that row must load as after the conversion.
    pub expected_device: Device,
    /// `(collection, state)`: one synced but unmarked, one marked. Both must
    /// come out exactly as they went in.
    pub versions: Vec<(&'static str, HashState)>,
    pub sync_key: AppStateSyncKey,
}

pub fn hash_state(version: u64, bootstrapped: bool, mac_mismatch_fatal: bool) -> HashState {
    HashState {
        version,
        hash: [0x2D; 128],
        index_value_map: HashMap::from([("idx".to_string(), vec![9u8, 8])]),
        mac_mismatch_fatal,
        bootstrapped,
    }
}

pub fn legacy_account() -> LegacyAccount {
    LegacyAccount {
        device_bincode: DEVICE_BINCODE,
        expected_device: decode_device(DEVICE_PB).unwrap(),
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

/// A legacy SQLite store: SQL migrations only (so the marker reads 'bincode'),
/// one account, its blobs written as bincode. `garbage_device` adds a second
/// account whose device blob is not bincode at all.
pub async fn legacy_sqlite(
    legacy: &LegacyAccount,
    garbage_device: bool,
) -> (String, tempfile::TempDir, i32) {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("legacy.db").display()
    );
    let pool = raw_sqlite(&url).await;
    sql::SQLITE_MIGRATOR.run(&pool).await.unwrap();
    let accounts = SqlStore::from_pool(SqlPool::Sqlite(sql::connect_sqlite(&url).await.unwrap()));
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
    accounts
        .close()
        .await
        .expect("close the store that made the accounts");
    pool.close().await;
    (url, dir, device_id)
}

/// A plain pool on the file, for the seeding and the probes below the store
/// (#173): the store's own readers are read-only, and these tests write.
pub async fn raw_sqlite(url: &str) -> SqlitePool {
    SqlitePool::connect(url)
        .await
        .expect("open the sqlite file")
}

pub async fn insert_device_sqlite(pool: &SqlitePool, device_id: i32, blob: &[u8]) {
    sqlx::query("INSERT INTO device (device_id, data) VALUES (?, ?)")
        .bind(device_id)
        .bind(blob)
        .execute(pool)
        .await
        .unwrap();
}

pub async fn seed_sqlite(pool: &SqlitePool, device_id: i32, legacy: &LegacyAccount) {
    insert_device_sqlite(pool, device_id, legacy.device_bincode).await;
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

pub async fn sqlite_marker(pool: &SqlitePool) -> String {
    sqlx::query_scalar("SELECT format FROM blob_format WHERE id = 1")
        .fetch_one(pool)
        .await
        .unwrap()
}

pub async fn sqlite_device_blob(pool: &SqlitePool, device_id: i32) -> Vec<u8> {
    sqlx::query_scalar("SELECT data FROM device WHERE device_id = ?")
        .bind(device_id)
        .fetch_one(pool)
        .await
        .unwrap()
}
