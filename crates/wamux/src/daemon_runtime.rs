//! The daemon's tokio runtime, built by hand so its exit is bounded (#239).

use std::future::Future;
use std::time::{Duration, Instant};

/// How the runtime's blocking pool ended when the runtime shut down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockingPoolExit {
    /// Every blocking task finished within the bound.
    Drained,
    /// The bound ran out with blocking tasks still running; they were left
    /// behind and die with the process.
    Abandoned,
}

/// What `block_on_bounded` hands back: the future's output and how the
/// blocking pool ended.
#[derive(Debug)]
pub struct BoundedRun<T> {
    pub output: T,
    pub blocking_pool: BlockingPoolExit,
}

/// Run `future` to completion on a multi-thread runtime, then shut the runtime
/// down waiting at most `blocking_bound` for its blocking tasks.
pub fn block_on_bounded<F: Future>(
    future: F,
    blocking_bound: Duration,
) -> std::io::Result<BoundedRun<F::Output>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let output = runtime.block_on(future);
    // Dropping the runtime would join the blocking pool with no limit: a task
    // stuck on a stalled read never ends (#239).
    let started = Instant::now();
    runtime.shutdown_timeout(blocking_bound);
    // tokio does not say whether the timeout fired; hitting the bound is the
    // only signal that tasks were left behind.
    let blocking_pool = if started.elapsed() >= blocking_bound {
        BlockingPoolExit::Abandoned
    } else {
        BlockingPoolExit::Drained
    };
    Ok(BoundedRun {
        output,
        blocking_pool,
    })
}

#[cfg(test)]
#[path = "daemon_runtime_tests.rs"]
mod tests;
