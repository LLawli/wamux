//! #229: the batch after the engine rolled the transaction back on its own.
//!
//! SQLite answers an I/O error or a full disk inside a transaction by rolling
//! the whole transaction back, savepoint included, without telling anyone. The
//! job's `ROLLBACK TO` then fails ("no such savepoint"), and so did every job
//! after it while the batch kept believing its savepoint and transaction were
//! open. These tests drive that over the fake connection
//! (`FakeConn::abort_transaction`) and require the next job to start clean.

use std::time::Duration;

use futures::poll;

use super::fake::{batch, committed};

/// Long enough for any schedule here, short enough to fail a stuck batch fast.
const PATIENCE: Duration = Duration::from_secs(5);

#[tokio::test]
async fn a_failed_job_after_an_automatic_rollback_does_not_wedge_the_writer() {
    let (group, db) = batch(8);
    let mut a = group.begin_job().await.unwrap();
    a.write("a");
    a.abort_transaction();
    a.fail().await;

    let mut b = tokio::time::timeout(PATIENCE, group.begin_job())
        .await
        .expect("the next job got the lock")
        .expect("the next job starts after the automatic rollback");
    b.write("b");
    b.finish().await.expect("the next job commits");

    assert_eq!(committed(&db), ["b"], "a's write went with the rollback");
    assert!(!db.lock().unwrap().in_transaction, "nothing is left open");
}

#[tokio::test]
async fn a_finish_after_an_automatic_rollback_fails_and_the_next_job_commits() {
    let (group, db) = batch(8);
    let mut a = group.begin_job().await.unwrap();
    a.write("a");
    a.abort_transaction();
    let a_result = a.finish().await;

    assert!(
        a_result.is_err(),
        "a job whose transaction is gone did not commit"
    );
    let mut b = tokio::time::timeout(PATIENCE, group.begin_job())
        .await
        .expect("the next job got the lock")
        .expect("the next job starts after the automatic rollback");
    b.write("b");
    b.finish().await.expect("the next job commits");
    assert_eq!(committed(&db), ["b"]);
}

#[tokio::test]
async fn jobs_waiting_on_a_batch_lost_to_an_automatic_rollback_are_told_it_failed() {
    let (group, db) = batch(8);
    let mut a = group.begin_job().await.unwrap();
    let mut b = Box::pin(group.begin_job());
    assert!(poll!(&mut b).is_pending(), "b queues behind a");

    a.write("a");
    let mut a_done = Box::pin(a.finish());
    assert!(poll!(&mut a_done).is_pending(), "a waits for b's COMMIT");
    let mut b = b.await.unwrap();
    b.write("b");
    b.abort_transaction();
    b.fail().await;
    let a_result = tokio::time::timeout(PATIENCE, a_done)
        .await
        .expect("a is answered");

    assert!(
        a_result.is_err(),
        "a's write went with the rollback, a must not see Ok"
    );
    assert!(committed(&db).is_empty());
    let mut c = tokio::time::timeout(PATIENCE, group.begin_job())
        .await
        .expect("the next job got the lock")
        .expect("the next job starts after the automatic rollback");
    c.write("c");
    c.finish().await.expect("the next job commits");
    assert_eq!(committed(&db), ["c"]);
}
