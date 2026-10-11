//! The fake connection the group commit tests drive (#172, #173).
//!
//! It keeps the writes of the open transaction apart from the committed ones,
//! honors the savepoint, can be told to fail its COMMIT, and answers the way
//! SQLite does when a statement finds nothing to act on: COMMIT and ROLLBACK
//! outside a transaction fail, RELEASE and ROLLBACK TO without the savepoint
//! fail, and a SAVEPOINT outside a transaction opens one that its RELEASE
//! commits. It can also stall one statement (#173), either after running it
//! (sqlx-sqlite runs a queued statement even when the caller's future is gone)
//! or without running it (an engine that runs nothing once the future is gone).

use std::sync::{Arc, Mutex as StdMutex};

use wacore::store::error::{Result as StoreResult, StoreError};

use super::{BatchConn, ControlStatement, GroupCommit};

/// What a stalled statement does before it hangs forever.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stall {
    /// The statement takes effect, then the future never returns.
    RunsThenHangs,
    /// The future never returns and the statement never takes effect.
    HangsWithoutRunning,
}

#[derive(Default)]
pub(super) struct FakeDb {
    pub(super) log: Vec<&'static str>,
    pub(super) open_writes: Vec<String>,
    pub(super) committed: Vec<String>,
    pub(super) savepoint_mark: Option<usize>,
    pub(super) in_transaction: bool,
    /// The transaction was opened by a SAVEPOINT, so its RELEASE commits it.
    pub(super) implicit: bool,
    pub(super) fail_commit: bool,
    /// One statement to stall, once.
    pub(super) stall: Option<(ControlStatement, Stall)>,
}

pub(super) struct FakeConn(pub(super) Arc<StdMutex<FakeDb>>);

impl FakeConn {
    /// A job's own statement: inside a transaction it waits for the COMMIT.
    pub(super) fn write(&mut self, value: &str) {
        let mut db = self.0.lock().unwrap();
        if db.in_transaction {
            db.open_writes.push(value.to_string());
        } else {
            db.committed.push(value.to_string());
        }
    }

    pub(super) fn in_transaction(&self) -> bool {
        self.0.lock().unwrap().in_transaction
    }

    pub(super) fn open_writes(&self) -> Vec<String> {
        self.0.lock().unwrap().open_writes.clone()
    }
}

fn refused(message: &str) -> StoreResult<()> {
    Err(StoreError::Connection(message.to_string().into()))
}

impl FakeDb {
    fn commit_open(&mut self) {
        let writes = std::mem::take(&mut self.open_writes);
        self.committed.extend(writes);
        self.in_transaction = false;
        self.implicit = false;
        self.savepoint_mark = None;
    }

    fn rollback_open(&mut self) {
        self.open_writes.clear();
        self.in_transaction = false;
        self.implicit = false;
        self.savepoint_mark = None;
    }

    fn begin(&mut self) -> StoreResult<()> {
        if self.in_transaction {
            return refused("cannot start a transaction within a transaction");
        }
        self.in_transaction = true;
        Ok(())
    }

    fn savepoint(&mut self) -> StoreResult<()> {
        if !self.in_transaction {
            self.in_transaction = true;
            self.implicit = true;
        }
        self.savepoint_mark = Some(self.open_writes.len());
        Ok(())
    }

    fn release(&mut self) -> StoreResult<()> {
        if self.savepoint_mark.take().is_none() {
            return refused("no such savepoint: wamux_job");
        }
        if self.implicit {
            self.commit_open();
        }
        Ok(())
    }

    fn rollback_to(&mut self) -> StoreResult<()> {
        let Some(mark) = self.savepoint_mark else {
            return refused("no such savepoint: wamux_job");
        };
        self.open_writes.truncate(mark);
        Ok(())
    }

    fn commit(&mut self) -> StoreResult<()> {
        if !self.in_transaction {
            return refused("cannot commit - no transaction is active");
        }
        if self.fail_commit {
            return refused("disk full (fake)");
        }
        self.commit_open();
        Ok(())
    }

    fn rollback(&mut self) -> StoreResult<()> {
        if !self.in_transaction {
            return refused("cannot rollback - no transaction is active");
        }
        self.rollback_open();
        Ok(())
    }

    fn run(&mut self, statement: ControlStatement) -> StoreResult<()> {
        match statement {
            ControlStatement::Begin => self.begin(),
            ControlStatement::Savepoint => self.savepoint(),
            ControlStatement::Release => self.release(),
            ControlStatement::RollbackToSavepoint => self.rollback_to(),
            ControlStatement::Commit => self.commit(),
            ControlStatement::Rollback => self.rollback(),
        }
    }

    /// The stall set for this statement, taken so it fires once.
    fn take_stall(&mut self, statement: ControlStatement) -> Option<Stall> {
        match self.stall {
            Some((stalled, how)) if stalled == statement => {
                self.stall = None;
                Some(how)
            }
            _ => None,
        }
    }
}

#[async_trait::async_trait]
impl BatchConn for FakeConn {
    async fn control(&mut self, statement: ControlStatement) -> StoreResult<()> {
        let stall = {
            let mut db = self.0.lock().unwrap();
            db.log.push(statement.sql());
            let stall = db.take_stall(statement);
            if stall == Some(Stall::HangsWithoutRunning) {
                None
            } else {
                Some((db.run(statement), stall.is_some()))
            }
        };
        match stall {
            Some((outcome, false)) => outcome,
            // The std lock is released above: nothing is held while hanging.
            _ => std::future::pending().await,
        }
    }
}

pub(super) fn batch(cap: usize) -> (GroupCommit<FakeConn>, Arc<StdMutex<FakeDb>>) {
    let db = Arc::new(StdMutex::new(FakeDb::default()));
    (GroupCommit::new(FakeConn(db.clone()), cap), db)
}

pub(super) fn log(db: &Arc<StdMutex<FakeDb>>) -> Vec<&'static str> {
    db.lock().unwrap().log.clone()
}

pub(super) fn committed(db: &Arc<StdMutex<FakeDb>>) -> Vec<String> {
    db.lock().unwrap().committed.clone()
}

pub(super) fn commits(db: &Arc<StdMutex<FakeDb>>) -> usize {
    log(db).iter().filter(|sql| **sql == "COMMIT").count()
}
