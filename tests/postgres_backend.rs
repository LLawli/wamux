//! M2 gate: the Postgres backend round-trips state and isolates accounts by
//! `device_id`. Requires a running Postgres (docker container `wamux-pg`):
//! `DATABASE_URL=postgres://wamux:wamux@localhost:5433/wamux` (default below).

use bytes::Bytes;
use wacore::store::traits::{Backend, DeviceStore, SignalStore};
use wamux::storage::postgres::{self, Accounts, PgBackend};

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wamux:wamux@localhost:5433/wamux".into())
}

/// Compile-time proof that the four trait impls satisfy the umbrella `Backend`.
fn _assert_backend<T: Backend>() {}
const _: () = {
    let _ = _assert_backend::<PgBackend>;
};

#[tokio::test]
async fn device_id_isolation_and_roundtrip() {
    let pool = postgres::connect(&database_url(), 5)
        .await
        .expect("connect postgres (is the wamux-pg container up?)");
    postgres::run_migrations(&pool)
        .await
        .expect("run migrations");
    let accounts = Accounts::new(pool.clone());

    let a = accounts
        .create(Some(&format!("test-a-{}", uuid::Uuid::new_v4())))
        .await
        .expect("create account a");
    let b = accounts
        .create(Some(&format!("test-b-{}", uuid::Uuid::new_v4())))
        .await
        .expect("create account b");
    assert_ne!(a.device_id, b.device_id, "device_ids must differ");

    let ba = PgBackend::new(pool.clone(), a.device_id);
    let bb = PgBackend::new(pool.clone(), b.device_id);

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
    assert!(accounts.delete(a.uuid).await.unwrap());
    assert!(accounts.delete(b.uuid).await.unwrap());
}
