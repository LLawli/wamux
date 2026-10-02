//! Shutdown trigger and drain deadline (#35). The timed tests run on a paused
//! clock (#67): `timeout` and the grace elapse in virtual time, so they are
//! deterministic and wait no wall-clock time.

use super::*;

const SOON: Duration = Duration::from_millis(200);

#[tokio::test(start_paused = true)]
async fn a_waiter_wakes_when_shutdown_fires() {
    let shutdown = Shutdown::new();
    let waiter = tokio::spawn({
        let shutdown = shutdown.clone();
        async move { shutdown.triggered().await }
    });
    assert!(!shutdown.is_triggered());
    shutdown.trigger();
    tokio::time::timeout(SOON, waiter).await.unwrap().unwrap();
}

// A stream opened during the drain must end too, so a late waiter cannot
// be allowed to miss a trigger that already happened.
#[tokio::test(start_paused = true)]
async fn a_waiter_that_arrives_late_still_sees_the_trigger() {
    let shutdown = Shutdown::new();
    shutdown.trigger();
    shutdown.trigger();
    tokio::time::timeout(SOON, shutdown.triggered())
        .await
        .expect("an already-fired shutdown must resolve at once");
}

#[tokio::test]
async fn a_serve_that_finishes_returns_its_output() {
    let shutdown = Shutdown::new();
    let out = drain_or_deadline(async { 7 }, &shutdown, Duration::from_secs(60)).await;
    assert_eq!(out, Some(7));
}

// The backstop: something that never drains is dropped after the grace,
// and not before shutdown fires.
#[tokio::test(start_paused = true)]
async fn a_serve_that_never_drains_is_cut_off_after_the_grace() {
    let shutdown = Shutdown::new();
    shutdown.trigger();
    let out = tokio::time::timeout(
        SOON * 5,
        drain_or_deadline(std::future::pending::<()>(), &shutdown, SOON),
    )
    .await
    .expect("the grace must bound the wait");
    assert_eq!(out, None);
}

#[tokio::test(start_paused = true)]
async fn the_grace_only_starts_counting_at_the_trigger() {
    let shutdown = Shutdown::new();
    let pending = drain_or_deadline(std::future::pending::<()>(), &shutdown, SOON);
    assert!(
        tokio::time::timeout(SOON * 3, pending).await.is_err(),
        "without a trigger the server must keep serving"
    );
}
