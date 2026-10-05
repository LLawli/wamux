//! Many accounts writing at once on one engine (#106). The SQLite engine and
//! the Turso engine both serialize the process's writes through one
//! connection; Turso's alternative, a connection per writer, failed with
//! `Busy` after a 5 s busy_timeout in the probe. This pins that no account's
//! write fails or leaks into another account under concurrent load.

use std::sync::Arc;

use bytes::Bytes;
use wacore::store::traits::Backend;

use crate::harness::{self, Harness};

const ACCOUNTS: usize = 8;
const ROUNDS: usize = 25;

fn arc(s: &str) -> Arc<str> {
    Arc::from(s)
}

/// One account's load: batches, single-row writes and reads, interleaved.
async fn hammer(backend: Arc<dyn Backend>, account: usize) {
    for round in 0..ROUNDS {
        let tag = (account * ROUNDS + round) as u8;
        let sessions: Vec<(Arc<str>, Bytes)> = (0..4)
            .map(|i| (arc(&format!("s{i}.0")), Bytes::from(vec![tag; 64])))
            .collect();
        backend.put_sessions_batch(&sessions).await.unwrap();
        backend.put_identity("peer.0", [tag; 32]).await.unwrap();
        let read = backend
            .get_sessions_batch(&[arc("s0.0"), arc("s3.0")])
            .await
            .unwrap();
        assert_eq!(read.len(), 2, "account {account} round {round}");
        assert!(
            read.iter().all(|(_, r)| r[..] == [tag; 64][..]),
            "account {account} read another writer's session"
        );
    }
}

async fn accounts_write_concurrently(h: Harness) {
    let mut rows = Vec::new();
    for account in 0..ACCOUNTS {
        let row = h
            .storage
            .create_account(Some(&format!("concurrency/{account}")))
            .await
            .unwrap();
        rows.push(row);
    }
    let tasks: Vec<_> = rows
        .iter()
        .enumerate()
        .map(|(account, row)| {
            tokio::spawn(hammer(h.storage.device_backend(row.device_id), account))
        })
        .collect();
    for task in tasks {
        task.await.expect("an account's writer panicked");
    }
    for (account, row) in rows.iter().enumerate() {
        let last = (account * ROUNDS + ROUNDS - 1) as u8;
        let backend = h.storage.device_backend(row.device_id);
        assert_eq!(
            backend.load_identity("peer.0").await.unwrap(),
            Some([last; 32]),
            "account {account} must end on its own last write"
        );
        assert!(h.storage.delete_account(row.uuid).await.unwrap());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sqlite_concurrent_accounts_write_without_errors() {
    accounts_write_concurrently(harness::sqlite().await).await;
}

#[cfg(feature = "turso")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn turso_concurrent_accounts_write_without_errors() {
    accounts_write_concurrently(harness::turso().await).await;
}
