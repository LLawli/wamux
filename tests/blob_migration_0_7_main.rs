//! The 0.7.0 -> main store migration end to end (#30), on SQLite so it needs no
//! container: a store exactly as a 0.7.0 daemon left it does NOT load on main,
//! the runner plans without writing, applies in one go, and afterwards the
//! store loads through the ordinary trait path and a re-run has nothing to do.
//!
//! The per-blob conversion is pinned in `blob_migration_0_7_main/tests.rs`;
//! this file pins the runner and the claim that matters: the account loads.
#![cfg(feature = "migrate-0-7-main")]

use wacore::store::traits::{AppSyncStore, DeviceStore};
use wamux::storage::StorageEngine;
use wamux::storage::blob_migration_0_7_main::MIGRATE_0_7_MAIN;
use wamux::storage::blob_migration_runner::{apply_sqlite, build_plan_sqlite};

#[allow(dead_code)]
mod common;

const COLLECTION: &str = "regular_high";
const PN: &str = "559980000001@s.whatsapp.net";

fn encode<T: serde::Serialize>(value: &T) -> Vec<u8> {
    bincode::serde::encode_to_vec(value, bincode::config::standard()).unwrap()
}

/// Overwrite one account's blob columns with what a 0.7.0 daemon would have
/// written. Raw SQL on purpose: the trait path can only write main's layout.
async fn seed_released_store(pool: &sqlx::SqlitePool, device_id: i32) {
    let mut device = wacore070::store::Device::new();
    // unwrap: parsing a literal, well-formed JID.
    device.pn = Some(PN.parse().unwrap());
    device.lid_migrated = true;
    let state = wacore070::appstate::hash::HashState {
        version: 12,
        hash: [0x42; 128],
        index_value_map: Default::default(),
        mac_mismatch_fatal: false,
    };
    let key = wacore070::store::traits::AppStateSyncKey {
        key_data: vec![0x11; 32],
        fingerprint: vec![1, 2, 3],
        timestamp: 1_749_400_000,
    };

    sqlx::query("UPDATE device SET data = ? WHERE device_id = ?")
        .bind(encode(&device))
        .bind(device_id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO app_state_versions (name, state_data, device_id) VALUES (?, ?, ?)")
        .bind(COLLECTION)
        .bind(encode(&state))
        .bind(device_id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO app_state_keys (key_id, key_data, device_id) VALUES (?, ?, ?)")
        .bind(&b"key-1"[..])
        .bind(encode(&key))
        .bind(device_id)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_released_store_loads_on_main_after_the_migration() {
    let (storage, _dir) = common::sqlite_engine().await;
    let account = storage.create_account(Some("migrate")).await.unwrap();
    let backend = storage.device_backend(account.device_id);
    backend.create().await.unwrap();
    seed_released_store(storage.pool(), account.device_id).await;

    // The premise: without the migration the account does not load.
    assert!(
        backend.load().await.is_err(),
        "a 0.7.0 device blob must not load on main, or this migration is dead code"
    );

    // Planning writes nothing: the account still does not load after it.
    let plan = build_plan_sqlite(storage.pool(), &MIGRATE_0_7_MAIN)
        .await
        .unwrap();
    assert_eq!(plan.devices.len(), 1);
    assert_eq!(plan.versions.len(), 1);
    assert_eq!(plan.sync_keys_verified, 1);
    assert!(backend.load().await.is_err(), "a dry run must not write");

    apply_sqlite(storage.pool(), &plan).await.unwrap();

    let device = backend.load().await.unwrap().expect("device present");
    assert_eq!(
        device.pn.as_ref().map(ToString::to_string).as_deref(),
        Some(PN)
    );
    assert!(device.lid_migrated, "a 0.7.0 field must survive, not reset");
    let state = backend
        .get_version(COLLECTION)
        .await
        .unwrap()
        .expect("the collection's version must still be there");
    assert_eq!(state.version, 12);
    assert!(!state.bootstrapped);

    let again = build_plan_sqlite(storage.pool(), &MIGRATE_0_7_MAIN)
        .await
        .unwrap();
    assert!(again.nothing_to_write(), "a re-run must be a no-op");
    assert_eq!(again.devices_already_current, 1);
    assert_eq!(again.versions_already_current, 1);
}

/// All or nothing: one unreadable blob fails the plan, so nothing is applied
/// and the good rows are left exactly as they were.
#[tokio::test]
async fn one_unreadable_blob_fails_the_whole_plan() {
    let (storage, _dir) = common::sqlite_engine().await;
    let good = storage.create_account(Some("good")).await.unwrap();
    let bad = storage.create_account(Some("bad")).await.unwrap();
    for account in [&good, &bad] {
        storage
            .device_backend(account.device_id)
            .create()
            .await
            .unwrap();
    }
    seed_released_store(storage.pool(), good.device_id).await;
    sqlx::query("UPDATE device SET data = ? WHERE device_id = ?")
        .bind(&[0xFFu8, 0x00, 0xDE, 0xAD][..])
        .bind(bad.device_id)
        .execute(storage.pool())
        .await
        .unwrap();

    assert!(
        build_plan_sqlite(storage.pool(), &MIGRATE_0_7_MAIN)
            .await
            .is_err()
    );
    let untouched: Vec<u8> = sqlx::query_scalar("SELECT data FROM device WHERE device_id = ?")
        .bind(good.device_id)
        .fetch_one(storage.pool())
        .await
        .unwrap();
    assert!(
        wamux::storage::blob_migration::decode_whole::<wacore070::store::Device>(&untouched)
            .is_ok(),
        "the good row must still be the 0.7.0 blob"
    );
}
