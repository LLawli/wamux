//! #239: a blocking task that never returns (a ureq read on a stalled
//! connection, inside `spawn_blocking`) must not keep the process from exiting.
//! Dropping a tokio runtime waits for every running blocking task with no
//! deadline; `block_on_bounded` waits at most the bound and says which it was.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::{BlockingPoolExit, block_on_bounded};

#[test]
fn stalled_blocking_task_is_abandoned_after_the_bound() {
    let bound = Duration::from_millis(200);
    // Held until the assertions are done, so the blocking task cannot return
    // before the shutdown gives up on it.
    let (release, stalled) = mpsc::channel::<()>();
    let started = Instant::now();
    let run = block_on_bounded(
        async move {
            tokio::task::spawn_blocking(move || stalled.recv());
            "served"
        },
        bound,
    )
    .expect("build the runtime");
    let took = started.elapsed();

    assert_eq!(run.output, "served", "the future's output passes through");
    assert_eq!(
        run.blocking_pool,
        BlockingPoolExit::Abandoned,
        "a blocking task still running at the bound is abandoned"
    );
    assert!(
        took < Duration::from_secs(2),
        "the exit must be bounded by {bound:?}, took {took:?}"
    );
    drop(release);
}

#[test]
fn finished_blocking_work_drains_cleanly() {
    let bound = Duration::from_secs(5);
    let started = Instant::now();
    let run = block_on_bounded(
        async {
            // Not awaited: still running when the future returns, so the
            // shutdown is what waits for it.
            // not a sync point: the sleep is the blocking work the shutdown must drain, nothing waits on it
            tokio::task::spawn_blocking(|| std::thread::sleep(Duration::from_millis(100)));
        },
        bound,
    )
    .expect("build the runtime");
    let took = started.elapsed();

    assert_eq!(run.blocking_pool, BlockingPoolExit::Drained);
    assert!(
        took < Duration::from_secs(2),
        "a drained pool returns when the work ends, not at the {bound:?} bound: took {took:?}"
    );
}

/// `run_isolated` drives library futures with `Handle::block_on` from a
/// blocking thread, which only works when worker threads keep the I/O and time
/// drivers moving (#107).
#[test]
fn runtime_is_multi_thread() {
    let run = block_on_bounded(
        async { tokio::runtime::Handle::current().runtime_flavor() },
        Duration::from_secs(1),
    )
    .expect("build the runtime");
    assert_eq!(run.output, tokio::runtime::RuntimeFlavor::MultiThread);
}
