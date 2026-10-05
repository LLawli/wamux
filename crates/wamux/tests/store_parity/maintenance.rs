//! `DeviceStore::maintenance` (#104), which the keepalive calls roughly
//! hourly. On SQLite it refreshes statistics and truncates the WAL, which
//! otherwise only grows between checkpoints; on Postgres it is a no-op
//! (autovacuum does the upkeep), and must still succeed. On Turso (#106) it is
//! the WAL checkpoint alone.

use std::path::Path;
use std::sync::Arc;

use bytes::Bytes;
use wamux::storage::StorageEngine;
use wamux::storage::sql::SqlStore;

use crate::harness;

/// 50 sessions written, then `maintenance`: the `-wal` file must go to zero and
/// the rows must still read back.
async fn maintenance_truncates_the_wal(store: &dyn StorageEngine, dir: &Path) {
    let row = store.create_account(Some("maintenance")).await.unwrap();
    let backend = store.device_backend(row.device_id);
    let sessions: Vec<(Arc<str>, Bytes)> = (0..50)
        .map(|i| {
            (
                Arc::from(format!("addr-{i}.0")),
                Bytes::from(vec![i as u8; 512]),
            )
        })
        .collect();
    backend.put_sessions_batch(&sessions).await.unwrap();

    let wal = dir.join("maintenance.db-wal");
    let before = std::fs::metadata(&wal)
        .expect("the WAL exists after writes")
        .len();
    assert!(before > 0, "writes must have reached the WAL");
    backend.maintenance().await.expect("maintenance");
    let after = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
    assert_eq!(
        after, 0,
        "the WAL must be truncated ({before} bytes before)"
    );
    assert_eq!(
        backend
            .get_session("addr-7.0")
            .await
            .unwrap()
            .map(|b| b.len()),
        Some(512)
    );
}

#[tokio::test]
async fn sqlite_maintenance_truncates_the_wal() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("maintenance.db");
    let store = SqlStore::open_sqlite(&format!("sqlite://{}?mode=rwc", db.display()))
        .await
        .expect("open sqlite");
    maintenance_truncates_the_wal(&store, dir.path()).await;
}

/// Turso keeps the WAL too, and its `maintenance` is the checkpoint alone:
/// `optimize` and `analysis_limit` are accepted and do nothing on turso 0.8.1
/// (#106). Same writes, same claim as the SQLite half.
#[cfg(feature = "turso")]
#[tokio::test]
async fn turso_maintenance_truncates_the_wal() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("maintenance.db");
    let store = wamux::storage::turso::TursoStore::open(&format!("turso://{}", db.display()))
        .await
        .expect("open turso");
    maintenance_truncates_the_wal(&store, dir.path()).await;
}

#[tokio::test]
async fn postgres_maintenance_is_a_no_op_that_succeeds() {
    let h = harness::postgres().await;
    let t = h.two_accounts("maintenance").await;
    t.ba.put_session("addr.0", b"kept").await.unwrap();
    t.ba.maintenance().await.expect("maintenance");
    assert_eq!(
        t.ba.get_session("addr.0").await.unwrap().as_deref(),
        Some(&b"kept"[..])
    );
    h.drop_accounts(t).await;
}
