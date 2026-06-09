//! Bounded in-memory ring buffer for quick-reconnect event replay (best-effort,
//! not durable). Oldest events are dropped on overflow.

use std::collections::VecDeque;

use tokio::sync::Mutex;

use crate::proto::v1::EventEnvelope;

pub struct EventRing {
    inner: Mutex<VecDeque<EventEnvelope>>,
    capacity: usize,
}

impl EventRing {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            capacity,
        }
    }

    /// Append, dropping the oldest if at capacity. No-op when capacity is 0.
    pub async fn push(&self, event: EventEnvelope) {
        if self.capacity == 0 {
            return;
        }
        let mut queue = self.inner.lock().await;
        while queue.len() >= self.capacity {
            queue.pop_front();
        }
        queue.push_back(event);
    }

    /// Snapshot up to the last `n` buffered events (newest-last order).
    pub async fn snapshot(&self, n: usize) -> Vec<EventEnvelope> {
        let queue = self.inner.lock().await;
        let take = n.min(queue.len());
        queue.iter().skip(queue.len() - take).cloned().collect()
    }
}
