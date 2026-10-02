use crate::ContractError;

/// Monotonic work progress with a checked total.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Progress {
    completed: u64,
    total: u64,
}

impl Progress {
    /// Starts work with a non-zero total.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidProgress`] for a zero total.
    pub const fn new(total: u64) -> Result<Self, ContractError> {
        if total == 0 {
            Err(ContractError::InvalidProgress)
        } else {
            Ok(Self {
                completed: 0,
                total,
            })
        }
    }
    /// Advances progress without allowing it to exceed the total.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidProgress`] when the advance exceeds the
    /// total, or [`ContractError::ArithmeticOverflow`] on addition overflow.
    pub fn advance(&mut self, amount: u64) -> Result<(), ContractError> {
        let next = self
            .completed
            .checked_add(amount)
            .ok_or(ContractError::ArithmeticOverflow)?;
        if next > self.total {
            return Err(ContractError::InvalidProgress);
        }
        self.completed = next;
        Ok(())
    }
    /// Returns completed units.
    #[must_use]
    pub const fn completed(self) -> u64 {
        self.completed
    }
    /// Returns total units.
    #[must_use]
    pub const fn total(self) -> u64 {
        self.total
    }
}
