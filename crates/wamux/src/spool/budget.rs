//! The global spool budget (#79): the bytes every transfer in flight holds on
//! disk, summed, may not pass `media_spool_max_bytes`. A transfer that would
//! pass it is refused with `ResourceExhausted` instead of filling the disk.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::WamuxError;

/// The shared byte counter. 0 as the maximum means unlimited.
#[derive(Debug)]
pub struct SpoolBudget {
    max_bytes: u64,
    held: AtomicU64,
}

impl SpoolBudget {
    pub fn new(max_bytes: u64) -> Arc<Self> {
        Arc::new(Self {
            max_bytes,
            held: AtomicU64::new(0),
        })
    }

    /// Hold `bytes` until the reservation is dropped, or refuse when they do
    /// not fit.
    pub fn try_reserve(self: &Arc<Self>, bytes: u64) -> Result<SpoolReservation, WamuxError> {
        self.claim(bytes)?;
        Ok(SpoolReservation {
            budget: Arc::clone(self),
            held: bytes,
        })
    }

    /// Add `bytes` to the total, or change nothing. One atomic compare and
    /// swap, so two transfers racing for the last bytes cannot both win.
    fn claim(&self, bytes: u64) -> Result<(), WamuxError> {
        let claimed = self
            .held
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                let total = held.saturating_add(bytes);
                (self.max_bytes == 0 || total <= self.max_bytes).then_some(total)
            });
        claimed
            .map(|_| ())
            .map_err(|_| WamuxError::ResourceExhausted("media spool budget exhausted".to_string()))
    }

    /// The bytes held right now, by every reservation.
    pub fn in_flight(&self) -> u64 {
        self.held.load(Ordering::Acquire)
    }
}

/// Bytes held against a [`SpoolBudget`]; dropping it gives them back.
#[derive(Debug)]
pub struct SpoolReservation {
    budget: Arc<SpoolBudget>,
    held: u64,
}

impl SpoolReservation {
    /// Hold `bytes` more, or refuse and keep what is already held.
    pub fn try_grow(&mut self, bytes: u64) -> Result<(), WamuxError> {
        self.budget.claim(bytes)?;
        self.held += bytes;
        Ok(())
    }

    /// The bytes this reservation holds.
    pub fn bytes(&self) -> u64 {
        self.held
    }
}

impl Drop for SpoolReservation {
    fn drop(&mut self) {
        self.budget.held.fetch_sub(self.held, Ordering::AcqRel);
    }
}

#[cfg(test)]
#[path = "budget_tests.rs"]
mod tests;
