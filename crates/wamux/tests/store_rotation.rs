//! #166: `rotate_engine` re-seals every row of an encrypted store under a new
//! key, in one transaction, and scrubs the old blobs out of the file.
//!
//! The decisive check is the one #165 uses for the conversion: every secret is
//! written through the store traits, the store is rotated, and everything reads
//! back through the traits under the NEW key. A re-sealed blob only opens if the
//! rotation named its row exactly as the stores do.
//!
//! A failure part way is made the way `store_conversion.rs` makes it: a trigger
//! that aborts the UPDATE of the second account's sessions.

use std::path::Path;

use sqlx::SqlitePool;
use wamux::storage::sql::SqlStore;
use wamux::storage::{StorageEngine, StoreKey, rotate_engine};

#[allow(dead_code)]
mod common;

use common::store_secrets::*;

const KEY_C: &str = "ef";

/// Two accounts, every secret written through the traits.
async fn write_two_accounts(store: &dyn StorageEngine) -> [i32; 2] {
    let mut ids = [0; 2];
    for (slot, name) in ids.iter_mut().zip(["rot/a", "rot/b"]) {
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

/// Every non-empty blob of every sealed column, column by column.
async fn snapshot(raw: &dyn Raw) -> Vec<Vec<Vec<u8>>> {
    let mut columns = Vec::with_capacity(SEALED.len());
    for (table, column) in SEALED {
        let mut blobs = raw.blobs(table, column).await;
        blobs.sort();
        columns.push(blobs);
    }
    columns
}

fn row_counts(snapshot: &[Vec<Vec<u8>>]) -> Vec<usize> {
    snapshot.iter().map(Vec::len).collect()
}

fn sqlite_db(dir: &Path) -> std::path::PathBuf {
    dir.join("wamux.db")
}

/// An encrypted SQLite store with two accounts, closed.
async fn encrypted_sqlite(url: &str, key: &StoreKey) -> [i32; 2] {
    let store = SqlStore::open_sqlite_keyed(url, Some(key)).await.unwrap();
    let ids = write_two_accounts(&store).await;
    common::lite_pool(&store).close().await;
    ids
}

/// The blobs of a SQLite store opened with `key`, then closed.
async fn sqlite_snapshot(url: &str, key: &StoreKey) -> Vec<Vec<Vec<u8>>> {
    let store = SqlStore::open_sqlite_keyed(url, Some(key))
        .await
        .expect("opens with its key");
    let blobs = snapshot(&store).await;
    common::lite_pool(&store).close().await;
    blobs
}

async fn sqlite_open_error(url: &str, key: Option<&StoreKey>) -> String {
    match SqlStore::open_sqlite_keyed(url, key).await {
        Ok(store) => {
            common::lite_pool(&store).close().await;
            panic!("the store opened with key {key:?}, a refusal was expected");
        }
        Err(error) => error.to_string(),
    }
}

async fn raw_pool(url: &str) -> SqlitePool {
    SqlitePool::connect(&format!("{url}?mode=rw"))
        .await
        .unwrap()
}

/// Abort every UPDATE of `sessions` for one account.
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

fn assert_no_old_blob_in(dump: &[u8], old_blobs: &[Vec<Vec<u8>>]) {
    for (blobs, (table, column)) in old_blobs.iter().zip(SEALED) {
        for blob in blobs {
            assert!(
                !contains(dump, blob),
                "a blob of {table}.{column} sealed by the old key is still in the file"
            );
        }
    }
}

#[tokio::test]
async fn sqlite_rotation_reseals_every_row_under_the_new_key() {
    let dir = tempfile::tempdir().unwrap();
    let url = sqlite_url(dir.path());
    let ids = encrypted_sqlite(&url, &key(KEY_A)).await;
    let before = sqlite_snapshot(&url, &key(KEY_A)).await;

    let rotated = rotate_engine(&url, 1, &key(KEY_A), &key(KEY_B))
        .await
        .expect("rotates");
    assert_eq!(rotated, 2);

    let store = SqlStore::open_sqlite_keyed(&url, Some(&key(KEY_B)))
        .await
        .expect("opens with the new key");
    assert_eq!(store.encryption_state().await, "encrypted");
    assert_every_column_is_sealed(&store, &key(KEY_B)).await;
    assert_eq!(row_counts(&snapshot(&store).await), row_counts(&before));
    assert_both_read_back_and_rewrite(&store, ids).await;
    common::lite_pool(&store).close().await;
}

#[tokio::test]
async fn sqlite_a_store_rotated_twice_opens_with_the_newest_key_only() {
    let dir = tempfile::tempdir().unwrap();
    let url = sqlite_url(dir.path());
    let ids = encrypted_sqlite(&url, &key(KEY_A)).await;

    assert_eq!(
        rotate_engine(&url, 1, &key(KEY_A), &key(KEY_B))
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        rotate_engine(&url, 1, &key(KEY_B), &key(KEY_C))
            .await
            .unwrap(),
        2
    );

    for retired in [KEY_A, KEY_B] {
        let shown = sqlite_open_error(&url, Some(&key(retired))).await;
        assert!(shown.contains("does not match"), "{retired}: {shown}");
    }
    let keyless = sqlite_open_error(&url, None).await;
    assert!(keyless.contains("store_key_file"), "{keyless}");

    let store = SqlStore::open_sqlite_keyed(&url, Some(&key(KEY_C)))
        .await
        .expect("opens with the newest key");
    assert_every_column_is_sealed(&store, &key(KEY_C)).await;
    assert_both_read_back_and_rewrite(&store, ids).await;
    common::lite_pool(&store).close().await;
}

#[tokio::test]
async fn sqlite_rotation_refuses_a_wrong_old_key_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let url = sqlite_url(dir.path());
    encrypted_sqlite(&url, &key(KEY_A)).await;
    let before = sqlite_snapshot(&url, &key(KEY_A)).await;

    let refused = rotate_engine(&url, 1, &key(KEY_C), &key(KEY_B))
        .await
        .expect_err("a wrong old key is refused");
    assert!(refused.to_string().contains("does not match"), "{refused}");

    assert_eq!(sqlite_snapshot(&url, &key(KEY_A)).await, before);
}

#[tokio::test]
async fn sqlite_rotation_refuses_a_plaintext_store_and_a_new_key_equal_to_the_old() {
    let dir = tempfile::tempdir().unwrap();
    let plain_url = sqlite_url(dir.path());
    let store = SqlStore::open_sqlite(&plain_url).await.unwrap();
    write_two_accounts(&store).await;
    common::lite_pool(&store).close().await;
    let refused = rotate_engine(&plain_url, 1, &key(KEY_A), &key(KEY_B))
        .await
        .expect_err("a plaintext store is refused");
    assert!(refused.to_string().contains("not encrypted"), "{refused}");
    let store = SqlStore::open_sqlite(&plain_url)
        .await
        .expect("still plaintext");
    assert_eq!(store.encryption_state().await, "plaintext");
    common::lite_pool(&store).close().await;

    let other = tempfile::tempdir().unwrap();
    let url = sqlite_url(other.path());
    encrypted_sqlite(&url, &key(KEY_A)).await;
    let before = sqlite_snapshot(&url, &key(KEY_A)).await;
    let refused = rotate_engine(&url, 1, &key(KEY_A), &key(KEY_A))
        .await
        .expect_err("the same key is refused");
    assert!(refused.to_string().contains("already"), "{refused}");
    assert_eq!(sqlite_snapshot(&url, &key(KEY_A)).await, before);
}

#[tokio::test]
async fn sqlite_a_rotation_that_fails_part_way_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let url = sqlite_url(dir.path());
    let ids = encrypted_sqlite(&url, &key(KEY_A)).await;
    let before = sqlite_snapshot(&url, &key(KEY_A)).await;

    break_account(&url, ids[1]).await;
    rotate_engine(&url, 1, &key(KEY_A), &key(KEY_B))
        .await
        .expect_err("the second account fails to rotate");
    repair_account(&url).await;

    // The first account was rotated inside the same transaction, so it rolled
    // back with the second: every blob is the one sealed by the old key.
    assert_eq!(sqlite_snapshot(&url, &key(KEY_A)).await, before);
    let shown = sqlite_open_error(&url, Some(&key(KEY_B))).await;
    assert!(shown.contains("does not match"), "{shown}");

    assert_eq!(
        rotate_engine(&url, 1, &key(KEY_A), &key(KEY_B))
            .await
            .unwrap(),
        2
    );
    let store = SqlStore::open_sqlite_keyed(&url, Some(&key(KEY_B)))
        .await
        .expect("the retry rotated it");
    assert_every_column_is_sealed(&store, &key(KEY_B)).await;
    assert_both_read_back_and_rewrite(&store, ids).await;
    common::lite_pool(&store).close().await;
}

#[tokio::test]
async fn sqlite_rotation_leaves_no_blob_of_the_old_key_in_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let url = sqlite_url(dir.path());
    encrypted_sqlite(&url, &key(KEY_A)).await;
    let old_blobs = sqlite_snapshot(&url, &key(KEY_A)).await;
    assert!(old_blobs.iter().all(|blobs| !blobs.is_empty()));

    rotate_engine(&url, 1, &key(KEY_A), &key(KEY_B))
        .await
        .expect("rotates");

    assert_no_old_blob_in(&dump_of(&sqlite_db(dir.path())), &old_blobs);
}

#[tokio::test]
async fn sqlite_rotating_a_store_with_no_accounts_moves_only_the_mark() {
    let dir = tempfile::tempdir().unwrap();
    let url = sqlite_url(dir.path());
    let store = SqlStore::open_sqlite_keyed(&url, Some(&key(KEY_A)))
        .await
        .unwrap();
    common::lite_pool(&store).close().await;

    assert_eq!(
        rotate_engine(&url, 1, &key(KEY_A), &key(KEY_B))
            .await
            .unwrap(),
        0
    );

    let shown = sqlite_open_error(&url, Some(&key(KEY_A))).await;
    assert!(shown.contains("does not match"), "{shown}");
    let store = SqlStore::open_sqlite_keyed(&url, Some(&key(KEY_B)))
        .await
        .expect("opens with the new key");
    assert_eq!(store.encryption_state().await, "encrypted");
    common::lite_pool(&store).close().await;
}

#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_a_store_rotated_twice_opens_with_the_newest_key_only() {
    use wamux::storage::turso::TursoStore;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wamux.db");
    let url = format!("turso://{}", path.display());

    let store = TursoStore::open_keyed(&url, Some(&key(KEY_A)))
        .await
        .unwrap();
    let ids = write_two_accounts(&store).await;
    let old_blobs = snapshot(&store).await;
    store.close().await.expect("release the file");

    assert_eq!(
        rotate_engine(&url, 1, &key(KEY_A), &key(KEY_B))
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        rotate_engine(&url, 1, &key(KEY_B), &key(KEY_C))
            .await
            .unwrap(),
        2
    );
    assert_no_old_blob_in(&dump_of(&path), &old_blobs);

    for retired in [KEY_A, KEY_B] {
        let refused = TursoStore::open_keyed(&url, Some(&key(retired)))
            .await
            .err()
            .unwrap_or_else(|| panic!("{retired} still opens the store"));
        assert!(refused.to_string().contains("does not match"), "{refused}");
    }

    let store = TursoStore::open_keyed(&url, Some(&key(KEY_C)))
        .await
        .expect("opens with the newest key");
    assert_every_column_is_sealed(&store, &key(KEY_C)).await;
    assert_eq!(row_counts(&snapshot(&store).await), row_counts(&old_blobs));
    assert_both_read_back_and_rewrite(&store, ids).await;
    store.close().await.expect("release the file");
}

#[tokio::test]
async fn postgres_a_store_rotated_twice_opens_with_the_newest_key_only() {
    let (url, name) = throwaway_postgres().await;

    let store = SqlStore::open_postgres_keyed(&url, 2, Some(&key(KEY_A)))
        .await
        .unwrap();
    let ids = write_two_accounts(&store).await;
    let before = snapshot(&store).await;
    common::pg_pool(&store).close().await;

    assert_eq!(
        rotate_engine(&url, 2, &key(KEY_A), &key(KEY_B))
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        rotate_engine(&url, 2, &key(KEY_B), &key(KEY_C))
            .await
            .unwrap(),
        2
    );

    for retired in [KEY_A, KEY_B] {
        let refused = SqlStore::open_postgres_keyed(&url, 2, Some(&key(retired)))
            .await
            .err()
            .unwrap_or_else(|| panic!("{retired} still opens the store"));
        assert!(refused.to_string().contains("does not match"), "{refused}");
    }

    let store = SqlStore::open_postgres_keyed(&url, 2, Some(&key(KEY_C)))
        .await
        .expect("opens with the newest key");
    assert_every_column_is_sealed(&store, &key(KEY_C)).await;
    assert_eq!(row_counts(&snapshot(&store).await), row_counts(&before));
    assert_both_read_back_and_rewrite(&store, ids).await;
    common::pg_pool(&store).close().await;
    drop_throwaway_postgres(&name).await;
}
