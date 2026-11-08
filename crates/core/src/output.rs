use crate::ContractError;

/// Supported export contracts; encoding is owned by the export crate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFormat {
    /// A still PNG image.
    Png,
    /// UTF-8 text rows.
    Text,
    /// An animated GIF of every source frame.
    Gif,
}

/// Owned output bytes whose allocation is released by RAII when dropped.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Output {
    format: OutputFormat,
    bytes: Vec<u8>,
}

impl Output {
    /// Creates output after checking its size against the supplied limit.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::ResourceLimitExceeded`] when the payload is
    /// larger than `max_bytes`.
    pub fn new(
        format: OutputFormat,
        bytes: Vec<u8>,
        max_bytes: u64,
    ) -> Result<Self, ContractError> {
        if u64::try_from(bytes.len()).map_err(|_| ContractError::ArithmeticOverflow)? > max_bytes {
            return Err(ContractError::ResourceLimitExceeded);
        }
        Ok(Self { format, bytes })
    }
    /// Returns the output format.
    #[must_use]
    pub const fn format(&self) -> OutputFormat {
        self.format
    }
    /// Borrows the encoded payload without copying it.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Cancellation, Dimensions, Generation, Progress, ResourceLimits, Sampling, Settings,
    };

    #[test]
    fn validates_contract_boundaries() {
        assert!(Dimensions::new(0, 1).is_err());
        assert!(Dimensions::new(u32::MAX, u32::MAX).is_ok());
        assert!(ResourceLimits::default()
            .validate(Dimensions::new(10_000, 5_001).unwrap())
            .is_err());
        assert!(ResourceLimits::new(50_000_001, 1).is_err());
        assert!(ResourceLimits::new(50_000_000, 1)
            .unwrap()
            .validate(Dimensions::new(5_000, 10_000).unwrap())
            .is_ok());
        assert!(Settings::new(4, 128, Sampling::Classic, "01").is_err());
        assert!(Settings::new(31, 128, Sampling::Classic, "01").is_err());
        assert!(Settings::new(5, 128, Sampling::Classic, "").is_err());
        assert!(Settings::new(5, 128, Sampling::Classic, "x".repeat(65)).is_err());
        assert!(Settings::new(5, 128, Sampling::Classic, "\n").is_err());
        assert!(Settings::new(5, 128, Sampling::Classic, "é").is_err());
        assert!(Progress::new(2).unwrap().advance(3).is_err());
        assert!(Progress::new(0).is_err());
        assert!(Output::new(OutputFormat::Text, vec![1, 2], 1).is_err());
    }

    #[test]
    fn cancellation_rejects_stale_and_cancelled_generations() {
        let token = Cancellation::new(Generation::new(7));
        assert!(token.check(Generation::new(7)).is_ok());
        assert!(matches!(
            token.check(Generation::new(8)),
            Err(ContractError::StaleGeneration)
        ));
        token.cancel();
        assert!(matches!(
            token.check(Generation::new(7)),
            Err(ContractError::Cancelled)
        ));

        let max_token = Cancellation::new(Generation::new(u64::MAX));
        assert!(!max_token.is_cancelled());
        max_token.cancel();
        assert!(max_token.is_cancelled());
        assert!(max_token.check(Generation::new(u64::MAX)).is_err());

        let zero_token = Cancellation::new(Generation::new(0));
        assert!(!zero_token.is_cancelled());
    }
}
