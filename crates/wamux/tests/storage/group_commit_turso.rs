//! #172: group commit on the turso engine, through the store traits.
//!
//! The step-by-step behavior of the batch is pinned by the unit tests of
//! `storage::group_commit` over a fake connection. These run the real engine
//! under real concurrency and check what an account would notice: every write
//! that returned Ok reads back, a failed write fails alone, a dropped writer
//! blocks nobody, the WAL upkeep still runs, and concurrent writers did share
//! commits (`commit_stats`), which is the point of the change.
#![cfg(feature = "turso")]

use std::sync::Arc;
use std::time::Duration;

use wacore::store::traits::Backend;
use wamux::storage::StorageEngine;
use wamux::storage::turso::TursoStore;

/// Long enough for a loaded run on a slow disk, short enough to fail a stuck batch.
const PATIENCE: Duration = Duration::from_secs(60);

async fn open_store(dir: &tempfile::TempDir) -> TursoStore {
    let url = format!("turso://{}", dir.path().join("wamux.db").display());
    TursoStore::open(&url).await.expect("open the turso store")
}

async fn accounts(store: &TursoStore, count: usize) -> Vec<Arc<dyn Backend>> {
    let mut backends = Vec::with_capacity(count);
    for index in 0..count {
        let account = store
            .create_account(Some(&format!("gc/{index}")))
            .await
            .unwrap();
        backends.push(store.device_backend(account.device_id));
    }
    backends
}

fn session_bytes(account: usize, write: usize) -> Vec<u8> {
    format!("session {account}.{write}").into_bytes()
}

/// `writes` sessions, one after the other, each awaited before the next.
async fn write_sessions(backend: Arc<dyn Backend>, account: usize, writes: usize) {
    for write in 0..writes {
        let address = format!("peer{write}@s.whatsapp.net.0");
        backend
            .put_session(&address, &session_bytes(account, write))
            .await
            .expect("a write that must succeed");
    }
}

async fn assert_sessions_read_back(backend: &dyn Backend, account: usize, writes: usize) {
    for write in 0..writes {
        let address = format!("peer{write}@s.whatsapp.net.0");
        let stored = backend.get_session(&address).await.unwrap();
        assert_eq!(
            stored.as_deref(),
            Some(&session_bytes(account, write)[..]),
            "account {account}, write {write}"
        );
    }
}

/// Every account writing at once, each write waiting for its own commit.
async fn write_concurrently(backends: &[Arc<dyn Backend>], writes: usize) {
    let tasks: Vec<_> = backends
        .iter()
        .enumerate()
        .map(|(account, backend)| tokio::spawn(write_sessions(backend.clone(), account, writes)))
        .collect();
    for task in tasks {
        tokio::time::timeout(PATIENCE, task)
            .await
            .expect("the writers finished")
            .expect("a writer panicked");
    }
}

fn stats_of(store: &TursoStore) -> wamux::storage::CommitStats {
    store
        .commit_stats()
        .expect("the turso engine reports its commit stats")
}

fn assert_commits_were_shared(store: &TursoStore) {
    let stats = stats_of(store);
    assert!(
        stats.commits < stats.jobs,
        "concurrent writers never shared a commit: {stats:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn turso_concurrent_accounts_share_commits_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(&dir).await;
    let backends = accounts(&store, 20).await;

    write_concurrently(&backends, 30).await;

    for (account, backend) in backends.iter().enumerate() {
        assert_sessions_read_back(&**backend, account, 30).await;
    }
    let stats = stats_of(&store);
    assert!(stats.jobs >= 600, "every write is a job: {stats:?}");
    assert_commits_were_shared(&store);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn turso_a_lone_writer_commits_every_write_on_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(&dir).await;
    let backend = accounts(&store, 1).await.remove(0);
    let before = stats_of(&store);

    write_sessions(backend.clone(), 0, 20).await;

    let after = stats_of(&store);
    let jobs = after.jobs - before.jobs;
    let commits = after.commits - before.commits;
    assert!(jobs >= 20, "every write is a job: {before:?} -> {after:?}");
    assert_eq!(commits, jobs, "with one writer the batch is one job");
    assert_sessions_read_back(&*backend, 0, 20).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn turso_a_failed_write_inside_a_batch_does_not_fail_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(&dir).await;
    let backends = accounts(&store, 10).await;
    // No account has this device_id: every write of it breaks the foreign key.
    let ghost = store.device_backend(999_999);

    let ghost_writes = tokio::spawn({
        let ghost = ghost.clone();
        async move {
            let mut failures = 0;
            for write in 0..20 {
                let address = format!("ghost{write}@s.whatsapp.net.0");
                if ghost.put_session(&address, b"orphan").await.is_err() {
                    failures += 1;
                }
            }
            failures
        }
    });
    write_concurrently(&backends, 20).await;
    let failures = tokio::time::timeout(PATIENCE, ghost_writes)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(failures, 20, "every write of a missing account fails");
    for (account, backend) in backends.iter().enumerate() {
        assert_sessions_read_back(&**backend, account, 20).await;
    }
    for write in 0..20 {
        let address = format!("ghost{write}@s.whatsapp.net.0");
        assert_eq!(ghost.get_session(&address).await.unwrap(), None);
    }
    assert_commits_were_shared(&store);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn turso_dropped_writers_never_block_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(&dir).await;
    let backends = accounts(&store, 40).await;

    let writers: Vec<_> = backends
        .iter()
        .enumerate()
        .map(|(account, backend)| tokio::spawn(write_sessions(backend.clone(), account, 1_000)))
        .collect();
    // not a sync point: the writers are cut at whatever step they reached, on purpose
    tokio::time::sleep(Duration::from_millis(200)).await;
    for writer in &writers {
        writer.abort();
    }
    for writer in writers {
        let _ = writer.await;
    }

    let survivor = backends[0].clone();
    let last = tokio::time::timeout(
        PATIENCE,
        survivor.put_session("after@s.whatsapp.net.0", b"still writes"),
    )
    .await;
    assert!(
        matches!(last, Ok(Ok(()))),
        "a write after the drops: {last:?}"
    );
    assert_eq!(
        survivor
            .get_session("after@s.whatsapp.net.0")
            .await
            .unwrap()
            .as_deref(),
        Some(&b"still writes"[..])
    );
    assert_commits_were_shared(&store);
    drop((backends, survivor));
    store
        .close()
        .await
        .expect("no batch or job holds the connection");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn turso_maintenance_runs_while_writers_are_busy() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(&dir).await;
    let backends = accounts(&store, 10).await;

    let writers: Vec<_> = backends
        .iter()
        .enumerate()
        .map(|(account, backend)| tokio::spawn(write_sessions(backend.clone(), account, 50)))
        .collect();
    let upkeep = backends[0].clone();
    for _ in 0..10 {
        tokio::time::timeout(PATIENCE, upkeep.maintenance())
            .await
            .expect("the WAL upkeep finished")
            .expect("the WAL upkeep runs between batches, never inside one");
    }
    for writer in writers {
        tokio::time::timeout(PATIENCE, writer)
            .await
            .unwrap()
            .unwrap();
    }

    for (account, backend) in backends.iter().enumerate() {
        assert_sessions_read_back(&**backend, account, 50).await;
    }
    assert_commits_were_shared(&store);
}
