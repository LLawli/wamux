//! M2 gate, now engine-parametric: every storage engine must round-trip state
//! and isolate accounts by `device_id`, with identical observable behavior.
//!
//! The Postgres case needs the docker container (`DATABASE_URL`); the SQLite
//! case needs nothing — it builds a fresh database file in a temp dir.

use std::sync::Arc;

use bytes::Bytes;
use wacore::store::traits::Backend;
use wamux::storage::StorageEngine;
use wamux::storage::postgres::PgBackend;
use wamux::storage::sqlite::SqliteBackend;

// Only a subset of the shared helpers is used per test binary.
#[allow(dead_code)]
mod common;

/// Compile-time proof that each engine's four trait impls satisfy the umbrella
/// `Backend`. If an engine ever misses a method, this fails before any test runs.
fn _assert_backend<T: Backend>() {}
const _: () = {
    let _ = _assert_backend::<PgBackend>;
    let _ = _assert_backend::<SqliteBackend>;
};

/// The shared body: two accounts on one engine must never see each other's
/// Signal state, and every value must come back exactly as written.
async fn round_trips_and_isolates(storage: Arc<dyn StorageEngine>) {
    let a = storage
        .create_account(Some(&format!("test-a-{}", uuid::Uuid::new_v4())))
        .await
        .expect("create account a");
    let b = storage
        .create_account(Some(&format!("test-b-{}", uuid::Uuid::new_v4())))
        .await
        .expect("create account b");
    assert_ne!(a.device_id, b.device_id, "device_ids must differ");

    let ba = storage.device_backend(a.device_id);
    let bb = storage.device_backend(b.device_id);

    // Identities are scoped: A's identity is invisible to B.
    ba.put_identity("alice@s.whatsapp.net", [7u8; 32])
        .await
        .unwrap();
    assert_eq!(
        ba.load_identity("alice@s.whatsapp.net").await.unwrap(),
        Some([7u8; 32])
    );
    assert_eq!(
        bb.load_identity("alice@s.whatsapp.net").await.unwrap(),
        None
    );

    // Sessions round-trip raw bytes.
    ba.put_session("alice@s.whatsapp.net", b"session-blob")
        .await
        .unwrap();
    assert_eq!(
        ba.get_session("alice@s.whatsapp.net").await.unwrap(),
        Some(Bytes::from_static(b"session-blob"))
    );
    assert!(ba.has_session("alice@s.whatsapp.net").await.unwrap());
    assert!(!bb.has_session("alice@s.whatsapp.net").await.unwrap());

    // PreKeys: id/max scoping.
    ba.store_prekey(42, b"pk", false).await.unwrap();
    assert_eq!(
        ba.load_prekey(42).await.unwrap().as_deref(),
        Some(&b"pk"[..])
    );
    assert_eq!(ba.get_max_prekey_id().await.unwrap(), 42);
    assert_eq!(bb.get_max_prekey_id().await.unwrap(), 0);

    // Device blob round-trips and is isolated.
    assert!(!ba.exists().await.unwrap());
    ba.create().await.unwrap();
    assert!(ba.exists().await.unwrap());
    assert!(!bb.exists().await.unwrap());
    let dev = ba.load().await.unwrap().expect("device a present");
    ba.save(&dev).await.unwrap(); // re-save the loaded device must not error

    // Cleanup: cascade removes all scoped rows.
    assert!(storage.delete_account(a.uuid).await.unwrap());
    assert!(storage.delete_account(b.uuid).await.unwrap());
}

#[tokio::test]
async fn postgres_round_trips_and_isolates_by_device_id() {
    let storage = common::pg_engine(5).await;
    round_trips_and_isolates(storage).await;
}

#[tokio::test]
async fn sqlite_round_trips_and_isolates_by_device_id() {
    let (storage, _dir) = common::sqlite_engine().await;
    round_trips_and_isolates(storage).await;
}

/// Regression for the SQLite-only trap: `PRAGMA foreign_keys` defaults to OFF,
/// so without the explicit pragma in `sqlite::connect` this delete would report
/// success while leaving every scoped Signal row behind — a silent, permanent
/// leak of one account's key material into the file. Reaches for the raw pool
/// on purpose: the point is what survives *below* the trait.
#[tokio::test]
async fn sqlite_account_delete_cascades_to_scoped_rows() {
    let (storage, _dir) = common::sqlite_engine().await;
    let account = storage
        .create_account(Some("cascade-probe"))
        .await
        .expect("create account");
    storage
        .device_backend(account.device_id)
        .put_identity("alice@s.whatsapp.net", [3u8; 32])
        .await
        .expect("put identity");

    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM identities")
        .fetch_one(storage.pool())
        .await
        .expect("count before");
    assert_eq!(before, 1, "identity must be persisted before the delete");

    assert!(storage.delete_account(account.uuid).await.unwrap());

    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM identities")
        .fetch_one(storage.pool())
        .await
        .expect("count after");
    assert_eq!(after, 0, "cascade must remove the account's Signal rows");
}

/// The claim that makes the abstraction worth anything: both engines persist
/// the SAME bytes, so a store can move between them. Saves one identical
/// `Device` through each engine and compares the raw blob columns.
///
/// A divergence here (a bincode config drift, a driver encoding a BLOB
/// differently) would not fail any other test — each engine would keep reading
/// back what it wrote — but it would silently make the two stores
/// non-interchangeable. Needs both engines, so it runs in the Postgres pass.
#[tokio::test]
async fn both_engines_persist_byte_identical_device_blobs() {
    let pg = common::pg_engine(5).await;
    let (lite, _dir) = common::sqlite_engine().await;

    let pg_account = pg
        .create_account(Some(&format!("parity-{}", uuid::Uuid::new_v4())))
        .await
        .expect("create pg account");
    let lite_account = lite
        .create_account(Some("parity"))
        .await
        .expect("create sqlite account");

    // One Device, saved through both engines. `create()` mints random key
    // material, so the comparison is only meaningful on a single instance.
    let pg_backend = pg.device_backend(pg_account.device_id);
    pg_backend.create().await.expect("create device");
    let device = pg_backend.load().await.unwrap().expect("device present");
    lite.device_backend(lite_account.device_id)
        .save(&device)
        .await
        .expect("save through sqlite");

    let pg_blob: Vec<u8> = sqlx::query_scalar("SELECT data FROM device WHERE device_id = $1")
        .bind(pg_account.device_id)
        .fetch_one(pg.pool())
        .await
        .expect("read pg blob");
    let lite_blob: Vec<u8> = sqlx::query_scalar("SELECT data FROM device WHERE device_id = ?")
        .bind(lite_account.device_id)
        .fetch_one(lite.pool())
        .await
        .expect("read sqlite blob");

    assert!(!pg_blob.is_empty(), "device blob must not be empty");
    assert_eq!(
        pg_blob, lite_blob,
        "engines must persist identical device bytes"
    );

    assert!(pg.delete_account(pg_account.uuid).await.unwrap());
}
