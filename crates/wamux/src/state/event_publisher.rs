//! The one writer of an account's event stream (#213).
//!
//! Every envelope an account publishes, from the library or from the relay
//! itself (the send echo), goes through one queue drained by one task. That
//! task stamps `monotonic_seq`, pushes to the replay ring and broadcasts, in
//! that order and one envelope at a time, so subscribers see the sequence in
//! the order it was stamped and the ring never holds it out of order. A ring
//! out of order is not cosmetic: the replay dedupe in `event_service` skips
//! every live envelope at or below the highest replayed seq, so an envelope
//! that reached the ring late would be lost to a subscriber that joined in
//! between.
//!
//! The queue is unbounded on purpose. The library calls its handler inline on
//! its dispatch path and expects it never to wait; a bounded queue would have
//! to drop, and a relay cannot lose an event. The consumer only maps, stamps
//! and hands off in memory, never waits on a subscriber (the broadcast is
//! lossy for a slow one), so the queue drains as fast as events arrive.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use futures::FutureExt;
use tokio::sync::{mpsc, oneshot};
use wacore::types::events::Event;

use super::event_bridge::{self, EventCtx};
use crate::proto::v1::event_envelope::Event as WireEvent;

/// One queued publish. `Wire` carries the channel that confirms delivery; its
/// event is boxed because a wire event dwarfs the `Arc` of the other variant.
enum Job {
    Lib(Arc<Event>),
    Wire(Box<WireEvent>, oneshot::Sender<()>),
}

impl Job {
    fn kind(&self) -> &'static str {
        match self {
            Job::Lib(_) => "lib",
            Job::Wire(..) => "wire",
        }
    }
}

/// Handle to an account's publishing task. Cheap to clone; the task lives while
/// any clone does.
#[derive(Clone)]
pub struct EventPublisher {
    queue: mpsc::UnboundedSender<Job>,
}

impl EventPublisher {
    /// Spawn the task that owns `ctx`'s sequence, ring and broadcast. Needs a
    /// tokio runtime.
    pub fn spawn(ctx: EventCtx) -> Self {
        let (queue, jobs) = mpsc::unbounded_channel();
        tokio::spawn(drain(ctx, jobs));
        Self { queue }
    }

    /// The library path: enqueue and return. Never waits, never drops.
    pub fn publish_lib(&self, event: Arc<Event>) {
        // While any clone exists the receiver lives, so a send fails only if
        // the task died some other way (the runtime shutting down). Nothing
        // can be published then, and there is nobody to tell.
        let _ = self.queue.send(Job::Lib(event));
    }

    /// An event the relay made itself (the send echo). Returns once the
    /// envelope is in the ring and on the broadcast, so the caller can answer
    /// knowing a replay already sees it. If the task is gone (the runtime is
    /// shutting down) or the job panicked, it returns without publishing:
    /// best-effort, like the echo it carries.
    pub async fn publish_wire(&self, event: WireEvent) {
        let (done, confirmed) = oneshot::channel();
        if self.queue.send(Job::Wire(Box::new(event), done)).is_err() {
            return;
        }
        // An error means the job was dropped unconfirmed: the task died with
        // it queued, or it panicked while publishing. Same best-effort
        // outcome as above.
        let _ = confirmed.await;
    }
}

/// The owner task. Holds the `EventCtx` and not the `AccountHandle`: the
/// handle owns the publisher, so holding it here would be a cycle that keeps
/// the task alive forever. It ends when the last sender is dropped.
async fn drain(ctx: EventCtx, mut jobs: mpsc::UnboundedReceiver<Job>) {
    while let Some(job) = jobs.recv().await {
        let kind = job.kind();
        // A panic must not silence the whole account until a restart: the
        // mapping touches many modules, so a panic costs only its own event.
        // A panicked `Wire` job drops its oneshot, which releases the waiting
        // caller.
        let outcome = AssertUnwindSafe(run_job(&ctx, job)).catch_unwind().await;
        if outcome.is_err() {
            tracing::error!(account = %ctx.uuid, job = kind, "publishing an event panicked; skipping it");
        }
    }
}

async fn run_job(ctx: &EventCtx, job: Job) {
    match job {
        Job::Lib(event) => event_bridge::dispatch(ctx, &event).await,
        Job::Wire(event, done) => {
            event_bridge::publish_wire_event(ctx, *event).await;
            let _ = done.send(());
        }
    }
}

#[cfg(test)]
#[path = "event_publisher_tests.rs"]
mod tests;
