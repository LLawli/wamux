//! #172: the batch, driven step by step over a fake connection.
//!
//! The fake keeps the writes of the open transaction apart from the committed
//! ones, honors the savepoint, and can be told to fail its COMMIT, which no real
//! engine does on demand. The interleavings are made by polling futures by hand
//! (`futures::poll!`), so every test runs the same schedule every time.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use futures::poll;
use wacore::store::error::StoreError;

use super::*;

#[derive(Default)]
struct FakeDb {
    log: Vec<&'static str>,
    open_writes: Vec<String>,
    committed: Vec<String>,
    savepoint_mark: Option<usize>,
    in_transaction: bool,
    fail_commit: bool,
}

struct FakeConn(Arc<StdMutex<FakeDb>>);

impl FakeConn {
    /// A job's own statement: inside a transaction it waits for the COMMIT.
    fn write(&mut self, value: &str) {
        let mut db = self.0.lock().unwrap();
        if db.in_transaction {
            db.open_writes.push(value.to_string());
        } else {
            db.committed.push(value.to_string());
        }
    }

    fn in_transaction(&self) -> bool {
        self.0.lock().unwrap().in_transaction
    }

    fn open_writes(&self) -> Vec<String> {
        self.0.lock().unwrap().open_writes.clone()
    }
}

#[async_trait::async_trait]
impl BatchConn for FakeConn {
    async fn control(&mut self, statement: ControlStatement) -> StoreResult<()> {
        let mut db = self.0.lock().unwrap();
        db.log.push(statement.sql());
        match statement {
            ControlStatement::Begin => db.in_transaction = true,
            ControlStatement::Savepoint => db.savepoint_mark = Some(db.open_writes.len()),
            ControlStatement::Release => db.savepoint_mark = None,
            ControlStatement::RollbackToSavepoint => {
                let mark = db.savepoint_mark.expect("ROLLBACK TO without a savepoint");
                db.open_writes.truncate(mark);
            }
            ControlStatement::Commit if db.fail_commit => {
                return Err(StoreError::Connection("disk full (fake)".into()));
            }
            ControlStatement::Commit => {
                let writes = std::mem::take(&mut db.open_writes);
                db.committed.extend(writes);
                db.in_transaction = false;
            }
            ControlStatement::Rollback => {
                db.open_writes.clear();
                db.in_transaction = false;
            }
        }
        Ok(())
    }
}

fn batch(cap: usize) -> (GroupCommit<FakeConn>, Arc<StdMutex<FakeDb>>) {
    let db = Arc::new(StdMutex::new(FakeDb::default()));
    (GroupCommit::new(FakeConn(db.clone()), cap), db)
}

fn log(db: &Arc<StdMutex<FakeDb>>) -> Vec<&'static str> {
    db.lock().unwrap().log.clone()
}

fn committed(db: &Arc<StdMutex<FakeDb>>) -> Vec<String> {
    db.lock().unwrap().committed.clone()
}

fn commits(db: &Arc<StdMutex<FakeDb>>) -> usize {
    log(db).iter().filter(|sql| **sql == "COMMIT").count()
}

/// Long enough for any schedule here, short enough to fail a stuck batch fast.
const PATIENCE: Duration = Duration::from_secs(5);

#[tokio::test]
async fn a_lone_writer_commits_a_batch_of_one() {
    let (group, db) = batch(8);
    let mut job = group.begin_job().await.unwrap();
    job.write("a");
    job.finish().await.unwrap();

    assert_eq!(
        log(&db),
        [
            "BEGIN",
            "SAVEPOINT wamux_job",
            "RELEASE wamux_job",
            "COMMIT"
        ]
    );
    assert_eq!(committed(&db), ["a"]);
    assert_eq!(
        group.stats(),
        CommitStats {
            commits: 1,
            jobs: 1
        }
    );
    assert_eq!(group.pending_writers(), 0);
}

#[tokio::test]
async fn a_failed_job_rolls_back_to_its_savepoint_and_the_batch_carries_on() {
    let (group, db) = batch(8);
    let mut a = group.begin_job().await.unwrap();
    let mut b = Box::pin(group.begin_job());
    assert!(poll!(&mut b).is_pending(), "b queues behind a");
    assert_eq!(group.pending_writers(), 2);

    a.write("a");
    a.fail().await;
    let mut b = b.await.unwrap();
    b.write("b");
    b.finish().await.unwrap();

    assert_eq!(committed(&db), ["b"], "a's write was rolled back alone");
    assert_eq!(
        log(&db),
        [
            "BEGIN",
            "SAVEPOINT wamux_job",
            "ROLLBACK TO wamux_job",
            "RELEASE wamux_job",
            "SAVEPOINT wamux_job",
            "RELEASE wamux_job",
            "COMMIT",
        ],
        "b joined the batch a opened: one BEGIN, one COMMIT"
    );
}

#[tokio::test]
async fn a_failed_commit_fails_every_job_of_the_batch() {
    let (group, db) = batch(8);
    db.lock().unwrap().fail_commit = true;
    let mut a = group.begin_job().await.unwrap();
    let mut b = Box::pin(group.begin_job());
    assert!(poll!(&mut b).is_pending());

    a.write("a");
    let mut a_done = Box::pin(a.finish());
    assert!(poll!(&mut a_done).is_pending(), "a waits for b's COMMIT");
    let mut b = b.await.unwrap();
    b.write("b");
    let b_result = b.finish().await;
    let a_result = tokio::time::timeout(PATIENCE, a_done).await.unwrap();

    assert!(b_result.is_err(), "the job that ran COMMIT sees it fail");
    assert!(a_result.is_err(), "the job that waited sees it fail too");
    assert!(committed(&db).is_empty());
    assert_eq!(log(&db).last(), Some(&"ROLLBACK"));
}

#[tokio::test]
async fn a_job_whose_caller_went_away_after_it_ran_is_still_committed() {
    let (group, db) = batch(8);
    let mut a = group.begin_job().await.unwrap();
    let mut b = Box::pin(group.begin_job());
    assert!(poll!(&mut b).is_pending());

    a.write("a");
    let mut a_done = Box::pin(a.finish());
    assert!(poll!(&mut a_done).is_pending());
    drop(a_done);
    let mut b = b.await.unwrap();
    b.write("b");
    b.finish().await.unwrap();

    assert_eq!(committed(&db), ["a", "b"]);
    assert_eq!(
        group.stats(),
        CommitStats {
            commits: 1,
            jobs: 2
        }
    );
}

#[tokio::test]
async fn a_writer_dropped_while_queued_does_not_leave_the_batch_stuck() {
    let (group, db) = batch(8);
    let mut a = group.begin_job().await.unwrap();
    let mut b = Box::pin(group.begin_job());
    assert!(poll!(&mut b).is_pending());

    a.write("a");
    let mut a_done = Box::pin(a.finish());
    assert!(poll!(&mut a_done).is_pending(), "a waits: b is pending");
    // b goes away without ever running: nobody is left to commit a's batch
    // unless the batch notices.
    drop(b);
    assert_eq!(group.pending_writers(), 0);

    let a_result = tokio::time::timeout(PATIENCE, a_done).await;
    assert!(
        matches!(a_result, Ok(Ok(()))),
        "a was left waiting: {a_result:?}"
    );
    assert_eq!(committed(&db), ["a"]);
}

#[tokio::test]
async fn jobs_run_in_the_order_they_queued() {
    let (group, db) = batch(64);
    let mut first = group.begin_job().await.unwrap();
    let mut queued = Vec::new();
    for _ in 0..5 {
        let mut next = Box::pin(group.begin_job());
        assert!(poll!(&mut next).is_pending());
        queued.push(next);
    }
    first.write("0");
    let mut first_done = Box::pin(first.finish());
    assert!(poll!(&mut first_done).is_pending());

    // Each finish that still waits for its COMMIT is kept and awaited at the end;
    // the last job finds nobody pending and commits on its first poll.
    let mut waiting = vec![first_done];
    for (index, next) in queued.into_iter().enumerate() {
        let mut job = next.await.unwrap();
        job.write(&(index + 1).to_string());
        let mut done = Box::pin(job.finish());
        match poll!(&mut done) {
            std::task::Poll::Pending => waiting.push(done),
            std::task::Poll::Ready(result) => result.unwrap(),
        }
    }
    for done in waiting {
        tokio::time::timeout(PATIENCE, done).await.unwrap().unwrap();
    }

    assert_eq!(committed(&db), ["0", "1", "2", "3", "4", "5"]);
    assert_eq!(commits(&db), 1, "six queued jobs, one COMMIT");
}

#[tokio::test]
async fn the_batch_commits_when_it_reaches_its_cap() {
    let (group, db) = batch(2);
    let mut a = group.begin_job().await.unwrap();
    let mut b = Box::pin(group.begin_job());
    let mut c = Box::pin(group.begin_job());
    assert!(poll!(&mut b).is_pending());
    assert!(poll!(&mut c).is_pending());

    a.write("a");
    let mut a_done = Box::pin(a.finish());
    assert!(
        poll!(&mut a_done).is_pending(),
        "a waits: b and c are pending"
    );
    let mut b = b.await.unwrap();
    b.write("b");
    // b fills the batch (cap 2): it commits although c is still pending.
    b.finish().await.unwrap();
    assert_eq!(committed(&db), ["a", "b"]);
    tokio::time::timeout(PATIENCE, a_done)
        .await
        .unwrap()
        .unwrap();

    let mut c = c.await.unwrap();
    c.write("c");
    c.finish().await.unwrap();
    assert_eq!(committed(&db), ["a", "b", "c"]);
    assert_eq!(commits(&db), 2);
    assert_eq!(
        group.stats(),
        CommitStats {
            commits: 2,
            jobs: 3
        }
    );
}

#[tokio::test]
async fn an_abandoned_job_is_rolled_back_by_the_next_lock() {
    let (group, db) = batch(8);
    let mut a = group.begin_job().await.unwrap();
    let mut b = Box::pin(group.begin_job());
    assert!(poll!(&mut b).is_pending());

    a.write("a");
    let mut a_done = Box::pin(a.finish());
    assert!(poll!(&mut a_done).is_pending());
    // b runs half a job and is dropped (an early `?`, a client gone away).
    let mut b = b.await.unwrap();
    b.write("b-half");
    drop(b);

    let a_result = tokio::time::timeout(PATIENCE, a_done).await;
    assert!(
        matches!(a_result, Ok(Ok(()))),
        "a was stranded: {a_result:?}"
    );
    let read = group.lock().await;
    assert!(!read.in_transaction());
    drop(read);
    assert_eq!(committed(&db), ["a"], "b's half job never commits");
    assert!(log(&db).contains(&"ROLLBACK TO wamux_job"));
}

#[tokio::test]
async fn lock_autocommit_commits_an_open_batch_first() {
    let (group, db) = batch(8);
    let mut a = group.begin_job().await.unwrap();
    let mut upkeep = Box::pin(group.lock_autocommit());
    assert!(poll!(&mut upkeep).is_pending());
    let mut b = Box::pin(group.begin_job());
    assert!(poll!(&mut b).is_pending());

    a.write("a");
    let mut a_done = Box::pin(a.finish());
    assert!(poll!(&mut a_done).is_pending(), "a waits: b is pending");
    let upkeep = upkeep.await.unwrap();
    assert!(
        !upkeep.in_transaction(),
        "the upkeep runs outside the batch"
    );
    assert_eq!(committed(&db), ["a"], "the open batch was committed first");
    drop(upkeep);
    tokio::time::timeout(PATIENCE, a_done)
        .await
        .unwrap()
        .unwrap();

    let mut b = b.await.unwrap();
    b.write("b");
    b.finish().await.unwrap();
    assert_eq!(committed(&db), ["a", "b"]);
}

#[tokio::test]
async fn a_read_with_writers_queued_sees_the_open_batch() {
    let (group, db) = batch(8);
    let mut a = group.begin_job().await.unwrap();
    let mut read = Box::pin(group.lock());
    assert!(poll!(&mut read).is_pending());
    let mut b = Box::pin(group.begin_job());
    assert!(poll!(&mut b).is_pending());

    a.write("a");
    let mut a_done = Box::pin(a.finish());
    assert!(poll!(&mut a_done).is_pending());
    let read = read.await;
    assert!(read.in_transaction(), "b is pending: the batch stays open");
    assert_eq!(read.open_writes(), ["a"], "the read sees a's write");
    assert!(committed(&db).is_empty());
    drop(read);

    let mut b = b.await.unwrap();
    b.write("b");
    b.finish().await.unwrap();
    tokio::time::timeout(PATIENCE, a_done)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(committed(&db), ["a", "b"]);
    assert_eq!(commits(&db), 1);
}
