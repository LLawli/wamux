//! Putting the connection back in a known state after a control statement whose
//! future was dropped half way (#173).
//!
//! sqlx-sqlite 0.8.6 queues a statement on its worker thread and runs it even
//! when the caller's future is gone (`connection/worker.rs:354`), so the batch
//! can believe a transaction is open that already committed, or a savepoint
//! exists that was already released. The #172 module only narrowed that window.
//! An engine that runs nothing once the future is gone has the mirror case: the
//! batch believes a statement ran that did not.
//!
//! `BatchState::in_flight` marks the statement being awaited. Whoever takes the
//! lock next calls `GroupCommit::reconcile` before anything else. Every state
//! change here happens after the await it depends on, so a reconciliation that
//! is itself dropped leaves a mark for the next user to repeat it. Nobody is ever
//! told a COMMIT succeeded unless a COMMIT really ran and returned Ok.

use super::{BatchConn, BatchState, ControlStatement, GroupCommit, answer_waiters};

impl<C: BatchConn> GroupCommit<C> {
    /// Settle the statement a dropped future left in flight, if any.
    pub(super) async fn reconcile(&self, state: &mut BatchState<C>) {
        let Some(statement) = state.in_flight else {
            return;
        };
        tracing::warn!(
            statement = statement.sql(),
            "reconciling a dropped control statement"
        );
        match statement {
            ControlStatement::Begin | ControlStatement::Rollback => {
                state.discard_transaction().await
            }
            ControlStatement::Commit => self.settle_unknown_commit(state).await,
            ControlStatement::Savepoint
            | ControlStatement::Release
            | ControlStatement::RollbackToSavepoint => state.undo_savepoint().await,
        }
    }

    /// A COMMIT whose outcome nobody saw. The probe is a COMMIT of our own: Ok
    /// means the transaction was still open, so this one carried the jobs. An
    /// error means the earlier one committed, or an automatic rollback (IOERR,
    /// FULL) undid the batch: unknown, so the waiters are told so.
    async fn settle_unknown_commit(&self, state: &mut BatchState<C>) {
        if state.open.is_none() {
            state.in_flight = None;
            return;
        }
        if state.run_control(ControlStatement::Commit).await.is_ok() {
            let batch = state.open.take().unwrap_or_default();
            self.count_commit(batch.jobs as u64);
            answer_waiters(batch, Ok(()));
            return;
        }
        state.discard_transaction().await;
    }
}

impl<C: BatchConn> BatchState<C> {
    /// ROLLBACK, whatever the statement in flight was: no transaction is a
    /// normal outcome here. The open batch is dropped and its waiters told.
    async fn discard_transaction(&mut self) {
        if let Err(error) = self.run_control(ControlStatement::Rollback).await {
            tracing::debug!(%error, "no transaction to roll back while reconciling");
        }
        if let Some(batch) = self.open.take() {
            answer_waiters(batch, Err(UNKNOWN_OUTCOME.to_string()));
        }
        self.job_open = false;
    }

    /// Undo the job's savepoint. "no such savepoint" is the expected answer when
    /// the RELEASE did run: the job then stays in the batch and is committed
    /// with it, like any job that ran.
    async fn undo_savepoint(&mut self) {
        if let Err(error) = self.rollback_job().await {
            tracing::debug!(%error, "savepoint already gone while reconciling");
        }
        self.job_open = false;
    }
}

const UNKNOWN_OUTCOME: &str = "the batch's COMMIT was interrupted and its outcome is unknown; check the write before retrying";
