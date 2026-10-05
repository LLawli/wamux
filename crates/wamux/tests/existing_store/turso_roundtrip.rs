//! One file, both families, in turns (#106). The Turso engine applies
//! `migrations_sqlite/` and records them the way sqlx does, so a store moves
//! from `sqlite://` to `turso://` and back with nothing converted. The way
//! back is the exit if Turso ever misbehaves, which is why it is tested.
//!
//! Each side is closed before the other opens the file. Both hold the file
//! open in this one process otherwise, and closing either one's descriptor
//! drops the other's POSIX lock (reproduced on turso 0.8.1 as a lost commit).

use std::path::Path;

use sqlx::SqlitePool;
use wamux::storage::StorageEngine;
use wamux::storage::sql::{SqlPool, SqlStore};
use wamux::storage::turso::TursoStore;

fn sqlite_url(path: &Path) -> String {
    format!("sqlite://{}?mode=rwc", path.display())
}

fn turso_url(path: &Path) -> String {
    format!("turso://{}", path.display())
}

async fn close_sqlite(store: SqlStore) {
    if let SqlPool::Sqlite(pool) = store.pool() {
        pool.close().await;
    }
}

/// Create an account named `name`, give it one identity, and check every
/// account written so far: present, in `device_id` order, unique and
/// increasing, each still holding its own identity.
async fn add_and_check(engine: &dyn StorageEngine, name: &str, earlier: &[&str]) {
    let row = engine.create_account(Some(name)).await.unwrap();
    let key = [earlier.len() as u8 + 1; 32];
    engine
        .device_backend(row.device_id)
        .put_identity("peer.0", key)
        .await
        .unwrap();

    let rows = engine.list_accounts().await.unwrap();
    let names: Vec<&str> = rows
        .iter()
        .filter_map(|r| r.external_ref.as_deref())
        .collect();
    let mut expected = earlier.to_vec();
    expected.push(name);
    assert_eq!(names, expected, "every account, in device_id order");
    assert!(
        rows.windows(2).all(|w| w[0].device_id < w[1].device_id),
        "device_ids must be unique and increasing: {:?}",
        rows.iter().map(|r| r.device_id).collect::<Vec<_>>()
    );
    for (i, r) in rows.iter().enumerate() {
        let got = engine
            .device_backend(r.device_id)
            .load_identity("peer.0")
            .await
            .unwrap();
        assert_eq!(got, Some([i as u8 + 1; 32]), "account {:?}", r.external_ref);
    }
}

#[tokio::test]
async fn turso_and_sqlite_reopen_each_others_store() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("roundtrip.db");

    let lite = SqlStore::open_sqlite(&sqlite_url(&path)).await.unwrap();
    add_and_check(&lite, "r1", &[]).await;
    close_sqlite(lite).await;

    let turso = TursoStore::open(&turso_url(&path)).await.unwrap();
    add_and_check(&turso, "r2", &["r1"]).await;
    turso.close().await.unwrap();

    let lite = SqlStore::open_sqlite(&sqlite_url(&path)).await.unwrap();
    add_and_check(&lite, "r3", &["r1", "r2"]).await;
    close_sqlite(lite).await;

    let turso = TursoStore::open(&turso_url(&path)).await.unwrap();
    add_and_check(&turso, "r4", &["r1", "r2", "r3"]).await;
    turso.close().await.unwrap();

    let pool = SqlitePool::connect(&sqlite_url(&path)).await.unwrap();
    let check: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(check, "ok", "the file must stay a sound SQLite database");
    pool.close().await;
}

/// `(version, description, success, checksum)` of every applied migration.
type MigrationRow = (i64, String, bool, Vec<u8>);

async fn applied_migrations(path: &Path) -> Vec<MigrationRow> {
    let pool = SqlitePool::connect(&sqlite_url(path)).await.unwrap();
    let rows = sqlx::query_as(
        "SELECT version, description, success, checksum FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    pool.close().await;
    rows
}

/// The tables and indexes a store holds, Turso's own bookkeeping left out.
async fn schema_objects(path: &Path) -> Vec<(String, String)> {
    let pool = SqlitePool::connect(&sqlite_url(path)).await.unwrap();
    let rows = sqlx::query_as(
        "SELECT type, name FROM sqlite_schema
         WHERE name NOT LIKE 'sqlite_%' AND name NOT LIKE '__turso_internal%'
         ORDER BY type, name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    pool.close().await;
    rows
}

/// A store Turso created reads to sqlx as one sqlx created: same migration
/// rows (sqlx refuses to open a store whose recorded checksum differs from its
/// embedded migration), same tables and indexes, and sqlx opens it.
#[tokio::test]
async fn turso_records_migrations_as_sqlx_does() {
    let dir = tempfile::tempdir().unwrap();
    let by_sqlx = dir.path().join("sqlx.db");
    let by_turso = dir.path().join("turso.db");
    close_sqlite(SqlStore::open_sqlite(&sqlite_url(&by_sqlx)).await.unwrap()).await;
    let turso = TursoStore::open(&turso_url(&by_turso)).await.unwrap();
    turso.close().await.unwrap();

    let expected = applied_migrations(&by_sqlx).await;
    assert!(
        expected.len() >= 3,
        "sqlx applied {} migrations",
        expected.len()
    );
    assert_eq!(applied_migrations(&by_turso).await, expected);
    assert_eq!(
        schema_objects(&by_turso).await,
        schema_objects(&by_sqlx).await
    );

    let reopened = SqlStore::open_sqlite(&sqlite_url(&by_turso))
        .await
        .expect("sqlx opens the store Turso migrated");
    close_sqlite(reopened).await;
}
