//! #165: turning the key on over a store that already has accounts converts it,
//! an interrupted conversion resumes, and `decrypt_engine` turns it back.
//!
//! The decisive check is the same one for every engine: write every secret
//! through the store traits WITHOUT a key, convert, and read everything back
//! through the traits WITH it. A converted blob only opens if the conversion
//! named its row exactly as the stores do, so a wrong row key in any of the 13
//! columns fails here and not on a live account.
//!
//! Interruptions are made the way `store_parity/atomicity.rs` makes failures: a
//! trigger that aborts the write of the second account.

use std::path::Path;

use sqlx::SqlitePool;
use wamux::storage::sql::SqlStore;
use wamux::storage::{StorageEngine, StoreKey, decrypt_engine};

#[allow(dead_code)]
mod common;

use common::store_secrets::*;

/// Two accounts, every secret written through the traits.
async fn write_two_accounts(store: &dyn StorageEngine) -> [i32; 2] {
    let mut ids = [0; 2];
    for (slot, name) in ids.iter_mut().zip(["conv/a", "conv/b"]) {
        let account = store.create_account(Some(name)).await.unwrap();
        write_every_secret(&*store.device_backend(account.device_id)).await;
        *slot = account.device_id;
    }
    ids
}

/// Both accounts read back, then written again: reading consumes the sent
/// message, and the sealed-column check needs a row in every column.
async fn assert_both_read_back_and_rewrite(store: &dyn StorageEngine, ids: [i32; 2]) {
    for id in ids {
        let backend = store.device_backend(id);
        assert_every_secret_reads_back(&*backend).await;
        write_every_secret(&*backend).await;
    }
}

/// A plaintext SQLite store with two accounts, closed.
async fn plaintext_sqlite(url: &str) -> [i32; 2] {
    let store = SqlStore::open_sqlite(url).await.unwrap();
    let ids = write_two_accounts(&store).await;
    common::lite_pool(&store).close().await;
    ids
}

async fn raw_pool(url: &str) -> SqlitePool {
    SqlitePool::connect(&format!("{url}?mode=rw"))
        .await
        .unwrap()
}

async fn count(pool: &SqlitePool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

/// Abort every UPDATE of `sessions` for one account, which is what the
/// conversion (or the decrypt) of that account does.
async fn break_account(url: &str, device_id: i32) {
    let pool = raw_pool(url).await;
    sqlx::raw_sql(&format!(
        "CREATE TRIGGER stop_account BEFORE UPDATE ON sessions WHEN OLD.device_id = {device_id}
         BEGIN SELECT RAISE(ABORT, 'rejected by test'); END;"
    ))
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
}

async fn repair_account(url: &str) {
    let pool = raw_pool(url).await;
    sqlx::raw_sql("DROP TRIGGER stop_account")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

/// A plaintext store whose conversion was interrupted after the first account.
async fn interrupted_conversion(url: &str, key: &StoreKey) -> [i32; 2] {
    let ids = plaintext_sqlite(url).await;
    break_account(url, ids[1]).await;
    let failed = SqlStore::open_sqlite_keyed(url, Some(key)).await;
    assert!(failed.is_err(), "the second account must fail to convert");
    ids
}

/// An encrypted store with two accounts, closed.
async fn encrypted_sqlite(url: &str, key: &StoreKey) -> [i32; 2] {
    let store = SqlStore::open_sqlite_keyed(url, Some(key)).await.unwrap();
    let ids = write_two_accounts(&store).await;
    common::lite_pool(&store).close().await;
    ids
}

fn sqlite_url(dir: &Path) -> String {
    format!("sqlite://{}", dir.join("wamux.db").display())
}

#[tokio::test]
async fn sqlite_a_plaintext_store_with_accounts_is_converted_when_a_key_is_turned_on() {
    let dir = tempfile::tempdir().unwrap();
    let url = sqlite_url(dir.path());
    let db = dir.path().join("wamux.db");
    let ids = plaintext_sqlite(&url).await;
    assert!(
        contains(&dump_of(&db), MARKER),
        "the test starts from plaintext"
    );
    let key = key(KEY_A);

    let store = SqlStore::open_sqlite_keyed(&url, Some(&key))
        .await
        .expect("a key over a store with accounts converts it");

    assert_eq!(
        store.list_accounts().await.unwrap().len(),
        2,
        "accounts kept"
    );
    assert_both_read_back_and_rewrite(&store, ids).await;
    assert_every_column_is_sealed(&store, &key).await;
    assert_eq!(store.encryption_state().await, "encrypted");
    let progress = count(
        common::lite_pool(&store),
        "SELECT count(*) FROM store_conversion_progress",
    );
    assert_eq!(progress.await, 0, "progress is cleared when it is done");
    assert_dump_hides_the_secrets(&dump_of(&db));
}

#[tokio::test]
async fn sqlite_an_interrupted_conversion_resumes_without_sealing_twice() {
    let dir = tempfile::tempdir().unwrap();
    let url = sqlite_url(dir.path());
    let key = key(KEY_A);
    let ids = interrupted_conversion(&url, &key).await;

    let pool = raw_pool(&url).await;
    let done: Vec<i32> = sqlx::query_scalar("SELECT device_id FROM store_conversion_progress")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(done, vec![ids[0]], "only the first account was converted");
    let state: String = sqlx::query_scalar("SELECT state FROM store_encryption WHERE id = 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        state, "plaintext",
        "the store is not encrypted until all are"
    );
    pool.close().await;

    repair_account(&url).await;
    let store = SqlStore::open_sqlite_keyed(&url, Some(&key))
        .await
        .expect("the same key finishes the conversion");
    assert_both_read_back_and_rewrite(&store, ids).await;
    assert_every_column_is_sealed(&store, &key).await;
    assert_eq!(store.encryption_state().await, "encrypted");
    let progress = count(
        common::lite_pool(&store),
        "SELECT count(*) FROM store_conversion_progress",
    );
    assert_eq!(progress.await, 0);
}

#[tokio::test]
async fn sqlite_an_interrupted_conversion_refuses_to_start_without_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let url = sqlite_url(dir.path());
    interrupted_conversion(&url, &key(KEY_A)).await;
    repair_account(&url).await;

    let error = SqlStore::open_sqlite(&url)
        .await
        .err()
        .expect("half sealed: not servable without the key");
    let shown = error.to_string();
    assert!(shown.contains("interrupted"), "{shown}");
    assert!(shown.contains("store_key_file"), "{shown}");
}

#[tokio::test]
async fn sqlite_resuming_a_conversion_with_another_key_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let url = sqlite_url(dir.path());
    interrupted_conversion(&url, &key(KEY_A)).await;
    repair_account(&url).await;

    let error = SqlStore::open_sqlite_keyed(&url, Some(&key(KEY_B)))
        .await
        .err()
        .expect("the first account is sealed under the other key");
    assert!(error.to_string().contains("does not match"), "{error}");
}

#[tokio::test]
async fn sqlite_decrypt_turns_an_encrypted_store_back_into_plaintext() {
    let dir = tempfile::tempdir().unwrap();
    let url = sqlite_url(dir.path());
    let key = key(KEY_A);
    let ids = encrypted_sqlite(&url, &key).await;

    let decrypted = decrypt_engine(&url, 2, &key).await.expect("decrypt");
    assert_eq!(decrypted, 2, "both accounts");

    let store = SqlStore::open_sqlite(&url)
        .await
        .expect("opens with no key");
    assert_eq!(store.encryption_state().await, "plaintext");
    assert_eq!(
        store.blobs("sessions", "record").await,
        vec![MARKER.to_vec(), MARKER.to_vec()],
        "the stored bytes are the secrets themselves again"
    );
    assert_both_read_back_and_rewrite(&store, ids).await;
    let pool = common::lite_pool(&store);
    assert_eq!(
        count(pool, "SELECT count(*) FROM store_conversion_progress").await,
        0
    );
    let key_id: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT key_id FROM store_encryption WHERE id = 1")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(key_id, None, "a plaintext store names no key");
}

#[tokio::test]
async fn sqlite_an_interrupted_decrypt_refuses_a_normal_start_and_resumes_on_rerun() {
    let dir = tempfile::tempdir().unwrap();
    let url = sqlite_url(dir.path());
    let key = key(KEY_A);
    let ids = encrypted_sqlite(&url, &key).await;
    break_account(&url, ids[1]).await;
    assert!(
        decrypt_engine(&url, 2, &key).await.is_err(),
        "the second account fails"
    );

    let error = SqlStore::open_sqlite_keyed(&url, Some(&key))
        .await
        .err()
        .expect("half decrypted: not servable");
    let shown = error.to_string();
    assert!(shown.contains("interrupted"), "{shown}");
    assert!(shown.contains("wamux store decrypt"), "{shown}");

    repair_account(&url).await;
    let resumed = decrypt_engine(&url, 2, &key).await.expect("rerun finishes");
    assert_eq!(resumed, 1, "only the account that was left");
    let store = SqlStore::open_sqlite(&url).await.unwrap();
    assert_eq!(store.encryption_state().await, "plaintext");
    assert_both_read_back_and_rewrite(&store, ids).await;
}

#[tokio::test]
async fn sqlite_decrypt_refuses_a_plaintext_store_and_a_wrong_key_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let url = sqlite_url(dir.path());
    plaintext_sqlite(&url).await;
    let error = decrypt_engine(&url, 2, &key(KEY_A))
        .await
        .expect_err("plaintext");
    assert!(error.to_string().contains("not encrypted"), "{error}");

    let other = dir.path().join("other");
    std::fs::create_dir(&other).unwrap();
    let url = sqlite_url(&other);
    encrypted_sqlite(&url, &key(KEY_A)).await;
    let store = SqlStore::open_sqlite_keyed(&url, Some(&key(KEY_A)))
        .await
        .unwrap();
    let before = store.blobs("sessions", "record").await;
    common::lite_pool(&store).close().await;

    let error = decrypt_engine(&url, 2, &key(KEY_B))
        .await
        .expect_err("wrong key");
    assert!(error.to_string().contains("does not match"), "{error}");
    let store = SqlStore::open_sqlite_keyed(&url, Some(&key(KEY_A)))
        .await
        .unwrap();
    assert_eq!(
        store.blobs("sessions", "record").await,
        before,
        "bytes untouched"
    );
    assert_eq!(store.encryption_state().await, "encrypted");
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_a_plaintext_store_is_converted_and_decrypted_back() {
    use wamux::storage::turso::TursoStore;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wamux.db");
    let url = format!("turso://{}", path.display());
    let key = key(KEY_A);

    let store = TursoStore::open(&url).await.unwrap();
    let ids = write_two_accounts(&store).await;
    store.close().await.expect("release the file");

    let store = TursoStore::open_keyed(&url, Some(&key))
        .await
        .expect("converts");
    assert_both_read_back_and_rewrite(&store, ids).await;
    assert_every_column_is_sealed(&store, &key).await;
    assert_eq!(store.encryption_state().await, "encrypted");
    store.close().await.expect("release the file");
    assert_dump_hides_the_secrets(&dump_of(&path));

    // The experimental VACUUM rewrote the file: sqlx must still find it whole,
    // read every secret and write more.
    let lite_url = format!("sqlite://{}", path.display());
    let pool = raw_pool(&lite_url).await;
    let check: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(check, "ok", "the vacuumed file is intact");
    pool.close().await;
    let lite = SqlStore::open_sqlite_keyed(&lite_url, Some(&key))
        .await
        .expect("sqlx opens the file turso converted");
    assert_both_read_back_and_rewrite(&lite, ids).await;
    common::lite_pool(&lite).close().await;

    assert_eq!(decrypt_engine(&url, 1, &key).await.unwrap(), 2);
    let store = TursoStore::open(&url).await.expect("opens with no key");
    assert_eq!(store.encryption_state().await, "plaintext");
    assert_both_read_back_and_rewrite(&store, ids).await;
    store.close().await.expect("release the file");
}

#[tokio::test]
async fn postgres_a_plaintext_store_is_converted_and_decrypted_back() {
    let (url, name) = throwaway_postgres().await;
    let key = key(KEY_A);

    let store = SqlStore::open_postgres(&url, 2).await.unwrap();
    let ids = write_two_accounts(&store).await;
    common::pg_pool(&store).close().await;

    let store = SqlStore::open_postgres_keyed(&url, 2, Some(&key))
        .await
        .expect("converts");
    assert_both_read_back_and_rewrite(&store, ids).await;
    assert_every_column_is_sealed(&store, &key).await;
    assert_eq!(store.encryption_state().await, "encrypted");
    common::pg_pool(&store).close().await;

    assert_eq!(decrypt_engine(&url, 2, &key).await.unwrap(), 2);
    let store = SqlStore::open_postgres(&url, 2)
        .await
        .expect("opens with no key");
    assert_eq!(store.encryption_state().await, "plaintext");
    assert_both_read_back_and_rewrite(&store, ids).await;
    common::pg_pool(&store).close().await;
    drop_throwaway_postgres(&name).await;
}
