//! The statements one batch runs on its connection: BEGIN, a job's SAVEPOINT and
//! its RELEASE or rollback, and the COMMIT that answers every job (#169, #172).
//! Each runs through `run_control`, which marks it in flight while it is
//! awaited, so a future dropped mid-statement is reconciled by the next user
//! (`group_commit_reconcile.rs`, #173).

use wacore::store::error::Result as StoreResult;

use super::{BatchConn, BatchState, ControlStatement, OpenBatch, answer_waiters};

impl<C: BatchConn> BatchState<C> {
    pub(super) async fn start_job(&mut self) -> StoreResult<()> {
        self.settle_abandoned().await?;
        if self.open.is_none() {
            self.begin_batch().await?;
        }
        self.run_control(ControlStatement::Savepoint).await?;
        self.job_open = true;
        Ok(())
    }

    /// The batch is recorded only once BEGIN returned. A future dropped mid-BEGIN
    /// leaves `in_flight` set and the next user rolls back whatever it opened.
    async fn begin_batch(&mut self) -> StoreResult<()> {
        self.run_control(ControlStatement::Begin).await?;
        self.open = Some(OpenBatch::default());
        Ok(())
    }

    /// Run one control statement, marking it in flight while it is awaited.
    /// An `Err` also means the statement finished, so the mark is cleared.
    pub(super) async fn run_control(&mut self, statement: ControlStatement) -> StoreResult<()> {
        self.in_flight = Some(statement);
        let outcome = self.conn.control(statement).await;
        self.in_flight = None;
        outcome
    }

    /// Undo the savepoint of a job that was dropped without `finish` or `fail`.
    pub(super) async fn settle_abandoned(&mut self) -> StoreResult<()> {
        if !self.job_open {
            return Ok(());
        }
        tracing::warn!("rolling back a job abandoned inside the open batch");
        self.rollback_job_or_discard().await;
        Ok(())
    }

    /// Roll the job's savepoint back. When that fails the transaction is gone
    /// (the engine rolled it back on its own, #229), so the batch is discarded
    /// and `job_open` is cleared: no path keeps a savepoint the engine lost.
    /// `false` when the batch was discarded.
    pub(super) async fn rollback_job_or_discard(&mut self) -> bool {
        let Err(error) = self.rollback_job().await else {
            return true;
        };
        tracing::error!(%error, "rolling back a job to its savepoint failed; discarding the batch");
        self.discard_after_lost_savepoint().await;
        false
    }

    pub(super) async fn rollback_job(&mut self) -> StoreResult<()> {
        self.run_control(ControlStatement::RollbackToSavepoint)
            .await?;
        self.run_control(ControlStatement::Release).await?;
        self.job_open = false;
        Ok(())
    }

    /// Release the savepoint and count the job; the jobs of the batch so far.
    pub(super) async fn end_job(&mut self) -> StoreResult<usize> {
        if let Err(error) = self.run_control(ControlStatement::Release).await {
            // A savepoint that cannot be released must not reach the COMMIT.
            self.rollback_job_or_discard().await;
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
    pub(super) async fn commit_batch(&mut self) -> StoreResult<u64> {
        if self.open.is_none() {
            return Ok(0);
        }
        let committed = self.run_control(ControlStatement::Commit).await;
        if committed.is_err()
            && let Err(error) = self.run_control(ControlStatement::Rollback).await
        {
            tracing::error!(%error, "rolling back a failed group commit failed");
        }
        let batch = self.open.take().unwrap_or_default();
        let jobs = batch.jobs as u64;
        answer_waiters(
            batch,
            committed.as_ref().map(|_| ()).map_err(|e| e.to_string()),
        );
        committed.map(|_| jobs)
    }
}
