//! #173: a control statement whose future is dropped half way.
//!
//! sqlx-sqlite queues each statement on a worker thread and runs it even after
//! the caller's future is gone (`connection/worker.rs` in sqlx-sqlite 0.8.6), so
//! a writer aborted mid-COMMIT left the batch believing a transaction was open
//! that had already committed: the next job's SAVEPOINT opened one on its own,
//! its RELEASE committed it, and its COMMIT failed with "no transaction is
//! active" (5 runs in 150 of `sqlite_dropped_writers_never_block_the_rest`).
//! An engine that runs nothing once the future is gone has the mirror case.
//! Whoever uses the connection next must find it in a known state.

use std::time::Duration;

use futures::poll;

use super::fake::{Stall, batch, committed};
use super::*;

/// Long enough for any schedule here, short enough to fail a stuck batch fast.
const PATIENCE: Duration = Duration::from_secs(5);

fn stall_next(
    db: &std::sync::Arc<std::sync::Mutex<super::fake::FakeDb>>,
    statement: ControlStatement,
    how: Stall,
) {
    db.lock().unwrap().stall = Some((statement, how));
}

fn in_transaction(db: &std::sync::Arc<std::sync::Mutex<super::fake::FakeDb>>) -> bool {
    db.lock().unwrap().in_transaction
}

/// A job that writes `value` and must commit, within `PATIENCE`.
async fn next_job_commits(group: &GroupCommit<super::fake::FakeConn>, value: &str) {
    let mut job = tokio::time::timeout(PATIENCE, group.begin_job())
        .await
        .expect("the next job is not stuck")
        .expect("the next job starts");
    job.write(value);
    tokio::time::timeout(PATIENCE, job.finish())
        .await
        .expect("the next job is not stuck")
        .expect("the next job commits");
}

#[tokio::test]
async fn a_commit_that_ran_after_its_caller_dropped_does_not_fail_the_next_job() {
    let (group, db) = batch(8);
    let mut a = group.begin_job().await.unwrap();
    a.write("a");
    stall_next(&db, ControlStatement::Commit, Stall::RunsThenHangs);
    let mut a_done = Box::pin(a.finish());
    assert!(poll!(&mut a_done).is_pending(), "a's COMMIT hangs");
    drop(a_done);
    assert_eq!(committed(&db), ["a"], "the COMMIT took effect all the same");

    next_job_commits(&group, "b").await;

    assert_eq!(committed(&db), ["a", "b"]);
    assert!(!in_transaction(&db), "nothing is left open");
    assert_eq!(group.pending_writers(), 0);
}

#[tokio::test]
async fn a_commit_that_never_ran_is_committed_for_its_waiters() {
    let (group, db) = batch(8);
    let mut a = group.begin_job().await.unwrap();
    let mut b_start = Box::pin(group.begin_job());
    assert!(poll!(&mut b_start).is_pending(), "b queues behind a");
    a.write("a");
    let mut a_done = Box::pin(a.finish());
    assert!(poll!(&mut a_done).is_pending(), "a waits for b to commit");
    let mut b = b_start.await.unwrap();
    b.write("b");
    stall_next(&db, ControlStatement::Commit, Stall::HangsWithoutRunning);
    let mut b_done = Box::pin(b.finish());
    assert!(poll!(&mut b_done).is_pending(), "b's COMMIT hangs unrun");
    drop(b_done);

    tokio::time::timeout(PATIENCE, a_done)
        .await
        .expect("a is answered")
        .expect("the batch was still open, so a real COMMIT carried a's job");
    assert_eq!(committed(&db), ["a", "b"]);
    next_job_commits(&group, "c").await;
    assert_eq!(committed(&db), ["a", "b", "c"]);
}

#[tokio::test]
async fn a_begin_that_never_ran_does_not_strand_the_next_job() {
    let (group, db) = batch(8);
    stall_next(&db, ControlStatement::Begin, Stall::HangsWithoutRunning);
    let mut a_start = Box::pin(group.begin_job());
    assert!(poll!(&mut a_start).is_pending(), "a's BEGIN hangs unrun");
    drop(a_start);

    next_job_commits(&group, "b").await;

    assert_eq!(committed(&db), ["b"]);
    assert!(!in_transaction(&db), "nothing is left open");
    assert_eq!(group.pending_writers(), 0);
}

#[tokio::test]
async fn a_release_that_ran_after_its_caller_dropped_keeps_the_job_in_the_batch() {
    let (group, db) = batch(8);
    let mut a = group.begin_job().await.unwrap();
    a.write("a");
    stall_next(&db, ControlStatement::Release, Stall::RunsThenHangs);
    let mut a_done = Box::pin(a.finish());
    assert!(poll!(&mut a_done).is_pending(), "a's RELEASE hangs");
    drop(a_done);

    next_job_commits(&group, "b").await;

    assert_eq!(
        committed(&db),
        ["a", "b"],
        "a's job ran and was released: it is committed with the next batch"
    );
    assert!(!in_transaction(&db), "nothing is left open");
}
