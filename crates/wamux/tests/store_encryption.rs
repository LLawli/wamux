//! #164: encryption at rest for the key material in the store. The tests start
//! from the claim a leaked dump is not a takeover, so they look at what is
//! stored, not only at what the traits return: every sealed column starts with
//! the blob header, and the database file holds none of the known secrets.
//!
//! SQLite and Turso need no server. Postgres gets a throwaway database (the
//! `wamux_upgrade_` prefix `account_leftovers` sweeps), because the shared
//! `wamux` database would turn `encrypted` for good the first time a keyed
//! test touched it.

use bytes::Bytes;
use wacore::store::traits::TcTokenEntry;
use wamux::storage::StorageEngine;
use wamux::storage::sql::SqlStore;

#[allow(dead_code)]
mod common;

use common::store_secrets::*;

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

    drop_throwaway_postgres(&name).await;
}
