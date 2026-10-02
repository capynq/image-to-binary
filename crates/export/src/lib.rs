//! Export boundary contracts without codec or filesystem policy.

use image_to_binary_core::{ContractError, Output, OutputFormat, ResourceLimits};

/// Private owned export plan retaining the output limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExportPlan {
    format: OutputFormat,
    max_bytes: u64,
}

impl ExportPlan {
    /// Creates a bounded export plan from resource limits.
    #[must_use]
    pub const fn new(format: OutputFormat, limits: ResourceLimits) -> Self {
        Self {
            format,
            max_bytes: limits.max_output_bytes(),
        }
    }
    /// Returns the requested format.
    #[must_use]
    pub const fn format(self) -> OutputFormat {
        self.format
    }
    /// Wraps encoded bytes only after enforcing the configured limit.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::ResourceLimitExceeded`] when bytes exceed the
    /// plan's output ceiling.
    pub fn finish(self, bytes: Vec<u8>) -> Result<Output, ContractError> {
        Output::new(self.format, bytes, self.max_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_bounded_and_retains_format() {
        let limits = ResourceLimits::new(1, 2).unwrap();
        let plan = ExportPlan::new(OutputFormat::Text, limits);
        assert_eq!(
            plan.finish(vec![1, 2]).unwrap().format(),
            OutputFormat::Text
        );
        assert!(plan.finish(vec![1, 2, 3]).is_err());
    }
}
