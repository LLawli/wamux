//! #172: creating an account on turso is a write like any other, so it is a job
//! of the group commit and returns only after the COMMIT that carries it. Its
//! `INSERT ... RETURNING` used to run on the read path, where an open batch
//! would have acknowledged it before any COMMIT.
#![cfg(feature = "turso")]

use wamux::storage::StorageEngine;
use wamux::storage::turso::TursoStore;

fn url_in(dir: &tempfile::TempDir) -> String {
    format!("turso://{}", dir.path().join("wamux.db").display())
}

fn stats_of(store: &TursoStore) -> wamux::storage::CommitStats {
    store
        .commit_stats()
        .expect("the turso engine reports its commit stats")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn turso_create_account_is_a_job_of_the_batch() {
    let dir = tempfile::tempdir().unwrap();
    let store = TursoStore::open(&url_in(&dir)).await.unwrap();
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
