//! #173: group commit on the SQLite engine, through the store traits.
//!
//! The #172 set (`group_commit_turso.rs`) on SQLite: every write that returned
//! Ok reads back, a failed write fails alone, a dropped writer blocks nobody, the
//! WAL upkeep still runs, concurrent writers share commits. Then what is new on
//! this engine: the writer runs `synchronous = FULL`, and the reads run on a
//! read-only pool of their own, so they neither wait for an open batch nor see
//! what it has not committed.

use std::sync::Arc;
use std::time::Duration;

use wacore::store::error::StoreError;
use wacore::store::traits::Backend;
use wamux::storage::StorageEngine;
use wamux::storage::sql::{SqlPool, SqlStore, SqliteConn};

/// Long enough for a loaded run on a slow disk, short enough to fail a stuck batch.
const PATIENCE: Duration = Duration::from_secs(60);

/// A read that waited on the writer would sit here until the writer let go.
const A_READ_THAT_DOES_NOT_WAIT: Duration = Duration::from_secs(5);

fn url_in(dir: &tempfile::TempDir) -> String {
    format!("sqlite://{}", dir.path().join("wamux.db").display())
}

async fn open_store(dir: &tempfile::TempDir) -> SqlStore {
    SqlStore::open_sqlite(&url_in(dir))
        .await
        .expect("open the sqlite store")
}

fn sqlite_conn(store: &SqlStore) -> SqliteConn {
    match store.pool() {
        SqlPool::Sqlite(conn) => conn.clone(),
        SqlPool::Pg(_) => panic!("opened as sqlite, holds a postgres pool"),
    }
}

async fn accounts(store: &SqlStore, count: usize) -> Vec<Arc<dyn Backend>> {
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

fn stats_of(store: &SqlStore) -> wamux::storage::CommitStats {
    store
        .commit_stats()
        .expect("the sqlite engine reports its commit stats")
}

fn assert_commits_were_shared(store: &SqlStore) {
    let stats = stats_of(store);
    assert!(
        stats.commits < stats.jobs,
        "concurrent writers never shared a commit: {stats:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sqlite_concurrent_accounts_share_commits_and_read_back() {
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
async fn sqlite_a_lone_writer_commits_every_write_on_its_own() {
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
async fn sqlite_a_failed_write_inside_a_batch_does_not_fail_the_others() {
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
async fn sqlite_dropped_writers_never_block_the_rest() {
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
        .expect("no batch or job holds the writer");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sqlite_maintenance_runs_while_writers_are_busy() {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sqlite_create_account_is_a_job_of_the_batch() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(&dir).await;
    let before = stats_of(&store);

    for index in 0..3 {
        store
            .create_account(Some(&format!("acct/{index}")))
            .await
            .unwrap();
    }

    let after = stats_of(&store);
    assert_eq!(
        after.jobs - before.jobs,
        3,
        "each account insert is one job: {before:?} -> {after:?}"
    );
    assert_eq!(
        after.commits - before.commits,
        3,
        "a lone writer commits each one on its own"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sqlite_writer_runs_synchronous_full() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(&dir).await;
    let conn = sqlite_conn(&store);
    let mut writer = conn.lock().await;

    let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
        .fetch_one(&mut *writer)
        .await
        .unwrap();
    let journal: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&mut *writer)
        .await
        .unwrap();
    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&mut *writer)
        .await
        .unwrap();

    assert_eq!(synchronous, 2, "the writer runs synchronous = FULL (2)");
    assert_eq!(journal, "wal");
    assert_eq!(foreign_keys, 1, "account deletes must cascade");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sqlite_reads_neither_wait_for_nor_see_an_open_write() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(&dir).await;
    let account = store.create_account(Some("reader")).await.unwrap();
    let backend = store.device_backend(account.device_id);
    let address = "open@s.whatsapp.net.0";
    let conn = sqlite_conn(&store);

    let mut writer = conn.lock().await;
    sqlx::query("BEGIN").execute(&mut *writer).await.unwrap();
    sqlx::query("INSERT INTO sessions (address, record, device_id) VALUES ($1, $2, $3)")
        .bind(address)
        .bind(&b"not committed yet"[..])
        .bind(account.device_id)
        .execute(&mut *writer)
        .await
        .unwrap();

    let during = tokio::time::timeout(A_READ_THAT_DOES_NOT_WAIT, backend.get_session(address))
        .await
        .expect("a read does not wait for the writer");
    assert_eq!(
        during.unwrap(),
        None,
        "a read does not see an uncommitted write"
    );

    sqlx::query("COMMIT").execute(&mut *writer).await.unwrap();
    drop(writer);
    assert_eq!(
        backend.get_session(address).await.unwrap().as_deref(),
        Some(&b"not committed yet"[..]),
        "the write is seen once committed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sqlite_the_read_pool_refuses_writes() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(&dir).await;
    let conn = sqlite_conn(&store);

    let attempt = sqlx::query("CREATE TABLE probe_173 (x INTEGER)")
        .execute(conn.read_pool())
        .await;

    let error = attempt.expect_err("a write through the read pool must fail");
    assert!(
        error.to_string().contains("readonly"),
        "refused because the pool is read-only: {error}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sqlite_batch_cap_one_never_shares_a_commit() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqlStore::open_sqlite_with_batch_cap(&url_in(&dir), 1)
        .await
        .expect("open the sqlite store with a cap of one");
    let backends = accounts(&store, 10).await;
    let before = stats_of(&store);

    write_concurrently(&backends, 20).await;

    let after = stats_of(&store);
    let jobs = after.jobs - before.jobs;
    assert!(jobs >= 200, "every write is a job: {before:?} -> {after:?}");
    assert_eq!(
        after.commits - before.commits,
        jobs,
        "a cap of one commits every job on its own"
    );
    for (account, backend) in backends.iter().enumerate() {
        assert_sessions_read_back(&**backend, account, 20).await;
    }
}

#[tokio::test]
async fn sqlite_an_in_memory_dsn_is_refused() {
    for dsn in ["sqlite::memory:", "sqlite://wamux.db?mode=memory"] {
        let opened = SqlStore::open_sqlite(dsn).await;
        assert!(
            matches!(opened, Err(StoreError::InvalidConfig(_))),
            "{dsn}: the readers would each see a database of their own: {:?}",
            opened.err()
        );
    }
}
