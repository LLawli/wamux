//! M2 gate, written against the `StorageEngine` trait rather than a concrete
//! engine: any engine must round-trip state and isolate accounts by
//! `device_id`. Requires the docker Postgres (`DATABASE_URL`).

use std::sync::Arc;

use bytes::Bytes;
use wacore::store::traits::Backend;
use wamux::storage::StorageEngine;
use wamux::storage::postgres::PgBackend;

// Only a subset of the shared helpers is used per test binary.
#[allow(dead_code)]
mod common;

/// Compile-time proof that each engine's four trait impls satisfy the umbrella
/// `Backend`. If an engine ever misses a method, this fails before any test runs.
fn _assert_backend<T: Backend>() {}
const _: () = {
    let _ = _assert_backend::<PgBackend>;
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
