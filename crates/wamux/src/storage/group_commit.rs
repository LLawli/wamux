//! Group commit for the engines that write through one connection (#169, #172).
//!
//! The file engines serialize every write on one connection, so with
//! `synchronous = FULL` each write would be its own fsync and the accounts would
//! queue behind each other. A batch lets the writers that are queued at the same
//! moment share one transaction and one COMMIT, without a timer:
//!
//! - a writer queues (it counts as pending from that moment), takes the lock,
//!   joins the open batch transaction or begins one, and runs its job inside a
//!   SAVEPOINT, so a job that fails rolls back alone;
//! - the writer that ends its job with no other writer pending, or with the
//!   batch at its cap, runs COMMIT and answers every job of the batch; any other
//!   writer leaves the batch open, releases the lock and waits;
//! - nobody is answered before COMMIT, and a failed COMMIT fails the whole batch.
//!
//! Under light load the batch has one job and nothing waits. The lock is a FIFO
//! `tokio::sync::Mutex` held across `.await`, which keeps the writes in the order
//! they queued.
//!
//! The module knows nothing of a driver: `BatchConn` runs the control statements
//! and the engine runs its own statements on the guarded connection. Turso is the
//! first engine wired to it (#172); SQLite follows (#173).

use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use tokio::sync::oneshot::error::TryRecvError;
use tokio::sync::{Mutex, MutexGuard, Notify, oneshot};
use wacore::store::error::{Result as StoreResult, StoreError};

/// The most jobs one COMMIT carries when the engine does not say otherwise.
/// Measured with `commit_bench` on turso, NVMe under btrfs on LUKS, 30 writes per
/// account (#172). The cap only matters once more writers queue than it holds:
/// at 10 accounts every cap from 16 up gave ~5,700 writes/s (serial: 756). At
/// 100 accounts, in the runs without a disk stall: 16 -> 7,600 writes/s, 32 ->
/// 11,000, 64 -> 14,000 (p99 14 ms), 128 -> 17,000 (p99 7 ms), 256 -> 17,400.
/// 128 is where it flattens. It does not hold the lock longer: the lock is
/// released between jobs, only the transaction stays open.
pub const DEFAULT_BATCH_CAP: usize = 128;

/// The statements the batch runs on the connection, besides the jobs' own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlStatement {
    Begin,
    Savepoint,
    Release,
    RollbackToSavepoint,
    Commit,
    Rollback,
}

impl ControlStatement {
    /// The SQL of the statement. One savepoint name is enough: one job runs at a
    /// time, and its savepoint is released before the next job starts.
    pub fn sql(self) -> &'static str {
        match self {
            ControlStatement::Begin => "BEGIN",
            ControlStatement::Savepoint => "SAVEPOINT wamux_job",
            ControlStatement::Release => "RELEASE wamux_job",
            ControlStatement::RollbackToSavepoint => "ROLLBACK TO wamux_job",
            ControlStatement::Commit => "COMMIT",
            ControlStatement::Rollback => "ROLLBACK",
        }
    }
}

/// A connection the batch can drive.
#[async_trait::async_trait]
pub trait BatchConn: Send + 'static {
    /// Run one control statement.
    async fn control(&mut self, statement: ControlStatement) -> StoreResult<()>;
}

/// How many COMMITs carried how many jobs, since the store opened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommitStats {
    pub commits: u64,
    pub jobs: u64,
}

/// What a waiting job is told once its batch is over: the COMMIT's message on
/// failure, because `StoreError` is not `Clone` and every job gets its own error.
type BatchAnswer = Result<(), String>;

/// The jobs that ran in the open transaction and the callers waiting for its COMMIT.
#[derive(Default)]
struct OpenBatch {
    jobs: usize,
    waiters: Vec<oneshot::Sender<BatchAnswer>>,
}

/// What the lock protects: the connection and the batch open on it.
struct BatchState<C> {
    conn: C,
    open: Option<OpenBatch>,
    /// A job's SAVEPOINT is open. Set by `begin_job`, cleared by `finish` and
    /// `fail`; still set when the next user finds the lock means the job was
    /// dropped half way, and that user rolls the savepoint back.
    job_open: bool,
}

/// Writers that queued and have not decided yet, and the wake-up for the batches
/// waiting on them.
struct Queue {
    pending: AtomicUsize,
    /// Signalled when `pending` reaches zero, so a waiting job whose last
    /// possible committer went away can commit its own batch (#172).
    drained: Notify,
}

/// One writer's place in the queue. A drop guard, so a future dropped anywhere
/// (before the lock, after the lock was granted, mid-job) gives the place back.
struct PendingSlot<'a>(&'a Queue);

impl Queue {
    fn enter(&self) -> PendingSlot<'_> {
        self.pending.fetch_add(1, Ordering::AcqRel);
        PendingSlot(self)
    }
}

impl Drop for PendingSlot<'_> {
    fn drop(&mut self) {
        if self.0.pending.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.drained.notify_waiters();
        }
    }
}

/// The one connection of an engine, shared by every account, with its batch.
pub struct GroupCommit<C: BatchConn> {
    state: Mutex<BatchState<C>>,
    queue: Queue,
    cap: usize,
    commits: AtomicU64,
    jobs: AtomicU64,
}

/// Exclusive use of the connection for a read or for work that is not a job.
pub struct ConnGuard<'a, C: BatchConn> {
    guard: MutexGuard<'a, BatchState<C>>,
}

/// A job running inside the batch: the connection, inside its SAVEPOINT.
/// `finish` ends it and returns once its batch committed; `fail` rolls it back.
/// Dropped without either, it is abandoned and rolled back by the next user.
pub struct JobGuard<'a, C: BatchConn> {
    guard: MutexGuard<'a, BatchState<C>>,
    owner: &'a GroupCommit<C>,
    slot: PendingSlot<'a>,
}

impl<C: BatchConn> BatchState<C> {
    async fn start_job(&mut self) -> StoreResult<()> {
        self.settle_abandoned().await?;
        if self.open.is_none() {
            self.begin_batch().await?;
        }
        self.conn.control(ControlStatement::Savepoint).await?;
        self.job_open = true;
        Ok(())
    }

    /// The batch is marked open before BEGIN runs, so a future dropped mid-BEGIN
    /// does not leave a transaction nobody knows about.
    async fn begin_batch(&mut self) -> StoreResult<()> {
        self.open = Some(OpenBatch::default());
        let begun = self.conn.control(ControlStatement::Begin).await;
        if begun.is_err() {
            self.open = None;
        }
        begun
    }

    /// Undo the savepoint of a job that was dropped without `finish` or `fail`.
    async fn settle_abandoned(&mut self) -> StoreResult<()> {
        if !self.job_open {
            return Ok(());
        }
        tracing::warn!("rolling back a job abandoned inside the open batch");
        self.rollback_job().await
    }

    async fn rollback_job(&mut self) -> StoreResult<()> {
        self.conn
            .control(ControlStatement::RollbackToSavepoint)
            .await?;
        self.conn.control(ControlStatement::Release).await?;
        self.job_open = false;
        Ok(())
    }

    /// Release the savepoint and count the job; the jobs of the batch so far.
    async fn end_job(&mut self) -> StoreResult<usize> {
        if let Err(error) = self.conn.control(ControlStatement::Release).await {
            // A savepoint that cannot be released must not reach the COMMIT.
            let _ = self.rollback_job().await;
            return Err(error);
        }
        self.job_open = false;
        let batch = self.open.get_or_insert_with(OpenBatch::default);
        batch.jobs += 1;
        Ok(batch.jobs)
    }

    /// COMMIT the open batch and answer its waiters; the jobs it carried.
    /// A failed COMMIT is rolled back and fails every job. The batch is dropped
    /// from the state only once the COMMIT returned.
    async fn commit_batch(&mut self) -> StoreResult<u64> {
        if self.open.is_none() {
            return Ok(0);
        }
        let committed = self.conn.control(ControlStatement::Commit).await;
        if committed.is_err()
            && let Err(error) = self.conn.control(ControlStatement::Rollback).await
        {
            tracing::error!(%error, "rolling back a failed group commit failed");
        }
        let batch = self.open.take().unwrap_or_default();
        let answer: BatchAnswer = committed.as_ref().map(|_| ()).map_err(|e| e.to_string());
        for waiter in batch.waiters {
            // A waiter whose caller went away has dropped its receiver: its job
            // was committed (or failed) all the same.
            let _ = waiter.send(answer.clone());
        }
        committed.map(|_| batch.jobs as u64)
    }
}

/// `None`: the sender was dropped without an answer.
fn batch_answer(received: Option<BatchAnswer>) -> StoreResult<()> {
    match received {
        Some(Ok(())) => Ok(()),
        Some(Err(message)) => Err(StoreError::Connection(message.into())),
        None => Err(StoreError::Connection(
            "the group commit was dropped before this job's COMMIT".into(),
        )),
    }
}

impl<C: BatchConn> GroupCommit<C> {
    /// Wrap a connection; `cap` is the most jobs one COMMIT carries.
    pub fn new(conn: C, cap: usize) -> Self {
        Self {
            state: Mutex::new(BatchState {
                conn,
                open: None,
                job_open: false,
            }),
            queue: Queue {
                pending: AtomicUsize::new(0),
                drained: Notify::new(),
            },
            cap: cap.max(1),
            commits: AtomicU64::new(0),
            jobs: AtomicU64::new(0),
        }
    }

    /// Queue as a writer, take the lock, and start a job in the batch.
    pub async fn begin_job(&self) -> StoreResult<JobGuard<'_, C>> {
        // Counted before the wait for the lock: the writer ahead of this one
        // must see it coming to leave the batch open for it.
        let slot = self.queue.enter();
        let mut guard = self.state.lock().await;
        guard.start_job().await?;
        Ok(JobGuard {
            guard,
            owner: self,
            slot,
        })
    }

    /// The connection for a read. An abandoned job is rolled back first, and an
    /// open batch with no writer pending is committed for the jobs waiting on it;
    /// with writers pending the batch stays open and the read sees it.
    pub async fn lock(&self) -> ConnGuard<'_, C> {
        let mut guard = self.state.lock().await;
        if let Err(error) = guard.settle_abandoned().await {
            tracing::error!(%error, "rolling back an abandoned job failed");
        }
        if self.pending_writers() == 0
            && let Err(error) = self.close_batch(&mut guard).await
        {
            tracing::error!(%error, "committing the open batch before a read failed");
        }
        ConnGuard { guard }
    }

    /// The connection outside any transaction, for what cannot run inside one
    /// (a WAL checkpoint, VACUUM, migrations): an open batch is committed first.
    pub async fn lock_autocommit(&self) -> StoreResult<ConnGuard<'_, C>> {
        let mut guard = self.state.lock().await;
        guard.settle_abandoned().await?;
        self.close_batch(&mut guard).await?;
        Ok(ConnGuard { guard })
    }

    /// Writers queued or holding the lock that have not decided yet.
    pub fn pending_writers(&self) -> usize {
        self.queue.pending.load(Ordering::Acquire)
    }

    pub fn stats(&self) -> CommitStats {
        CommitStats {
            commits: self.commits.load(Ordering::Relaxed),
            jobs: self.jobs.load(Ordering::Relaxed),
        }
    }

    /// The connection back, to close it.
    pub fn into_inner(self) -> C {
        self.state.into_inner().conn
    }

    /// COMMIT the open batch, if any, and count it.
    async fn close_batch(&self, state: &mut BatchState<C>) -> StoreResult<()> {
        let jobs = state.commit_batch().await?;
        if jobs > 0 {
            self.commits.fetch_add(1, Ordering::Relaxed);
            self.jobs.fetch_add(jobs, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Wait for the COMMIT of a batch someone else will run. The wait also ends
    /// when the last writer that could commit it goes away: the writer that
    /// dropped (before or after the lock was granted to it) signals `drained`,
    /// and this waiter then takes the lock and commits the batch itself (#172).
    async fn wait_for_commit(&self, mut answer: oneshot::Receiver<BatchAnswer>) -> StoreResult<()> {
        loop {
            // Registered before the check, so a signal between the two is kept.
            let drained = self.queue.drained.notified();
            tokio::pin!(drained);
            drained.as_mut().enable();
            if let Some(result) = self.commit_if_drained(&mut answer).await {
                return result;
            }
            tokio::select! {
                received = &mut answer => return batch_answer(received.ok()),
                _ = &mut drained => {}
            }
        }
    }

    /// With no writer pending, this waiter's batch has no committer left: take
    /// the lock and commit it. `None` while writers are pending, or if they came
    /// back before the lock was granted.
    async fn commit_if_drained(
        &self,
        answer: &mut oneshot::Receiver<BatchAnswer>,
    ) -> Option<StoreResult<()>> {
        if self.pending_writers() != 0 {
            return None;
        }
        let mut state = self.state.lock().await;
        if self.pending_writers() == 0 {
            if let Err(error) = state.settle_abandoned().await {
                tracing::error!(%error, "rolling back an abandoned job failed");
            }
            // The outcome reaches this waiter through its own receiver below.
            let _ = self.close_batch(&mut state).await;
        }
        match answer.try_recv() {
            Err(TryRecvError::Empty) => None,
            Ok(received) => Some(batch_answer(Some(received))),
            Err(TryRecvError::Closed) => Some(batch_answer(None)),
        }
    }
}

impl<'a, C: BatchConn> JobGuard<'a, C> {
    /// End the job and wait until the COMMIT that carries it. A job that ran is
    /// committed even if this future is dropped while it waits: the Signal
    /// ratchet it advanced already moved in memory, so un-writing it would
    /// desynchronize the session from the peer. The batch is committed by
    /// whoever ends it, whether or not this caller is still listening.
    pub async fn finish(self) -> StoreResult<()> {
        let JobGuard {
            mut guard,
            owner,
            slot,
        } = self;
        let jobs = guard.end_job().await?;
        let others = owner.pending_writers().saturating_sub(1);
        if others == 0 || jobs >= owner.cap {
            return owner.close_batch(&mut guard).await;
        }
        let (sender, answer) = oneshot::channel();
        if let Some(batch) = guard.open.as_mut() {
            batch.waiters.push(sender);
        }
        drop(slot);
        drop(guard);
        owner.wait_for_commit(answer).await
    }

    /// Roll the job back to its savepoint; the rest of the batch carries on.
    pub async fn fail(self) {
        let JobGuard {
            mut guard,
            owner,
            slot: _slot,
        } = self;
        if let Err(error) = guard.rollback_job().await {
            tracing::error!(%error, "rolling back a failed job to its savepoint failed");
            return;
        }
        if owner.pending_writers().saturating_sub(1) == 0
            && let Err(error) = owner.close_batch(&mut guard).await
        {
            tracing::error!(%error, "committing the batch after a failed job failed");
        }
    }
}

impl<C: BatchConn> Deref for ConnGuard<'_, C> {
    type Target = C;
    fn deref(&self) -> &C {
        &self.guard.conn
    }
}

impl<C: BatchConn> DerefMut for ConnGuard<'_, C> {
    fn deref_mut(&mut self) -> &mut C {
        &mut self.guard.conn
    }
}

impl<C: BatchConn> Deref for JobGuard<'_, C> {
    type Target = C;
    fn deref(&self) -> &C {
        &self.guard.conn
    }
}

impl<C: BatchConn> DerefMut for JobGuard<'_, C> {
    fn deref_mut(&mut self) -> &mut C {
        &mut self.guard.conn
    }
}

#[cfg(test)]
#[path = "group_commit_tests.rs"]
mod tests;
