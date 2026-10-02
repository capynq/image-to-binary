use crate::ContractError;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// Monotonic job generation used to reject stale asynchronous results.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Generation(u64);

impl Generation {
    /// Creates a generation identifier.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    /// Returns the raw generation value for adapter serialization.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Cloneable cancellation state with RAII-owned shared storage.
#[derive(Clone, Debug)]
pub struct Cancellation {
    generation: Generation,
    cancelled: Arc<AtomicBool>,
}

impl Cancellation {
    /// Creates a token for one job generation.
    #[must_use]
    pub fn new(generation: Generation) -> Self {
        Self {
            generation,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
    /// Requests cancellation for this token's generation.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    /// Returns whether this generation was cancelled or is stale.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    /// Rejects a stale result or a cancelled job.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::StaleGeneration`] for another generation or
    /// [`ContractError::Cancelled`] after cancellation was requested.
    pub fn check(&self, generation: Generation) -> Result<(), ContractError> {
        if generation != self.generation {
            return Err(ContractError::StaleGeneration);
        }
        if self.is_cancelled() {
            Err(ContractError::Cancelled)
        } else {
            Ok(())
        }
    }
}
