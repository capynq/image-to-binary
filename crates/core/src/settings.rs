use crate::ContractError;

/// Sampling behavior; codecs and rendering are implemented in later phases.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Sampling {
    Classic,
    Average,
}

/// Deterministic settings shared by preview and export.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Settings {
    cell_size: u8,
    threshold: u8,
    sampling: Sampling,
    ramp: String,
}

impl Settings {
    /// Creates validated settings. Cell sizes intentionally share the 5..=30 UI contract.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidSettings`] for an out-of-range cell
    /// size, or [`ContractError::InvalidRamp`] for an invalid ramp.
    pub fn new(
        cell_size: u8,
        threshold: u8,
        sampling: Sampling,
        ramp: impl Into<String>,
    ) -> Result<Self, ContractError> {
        if !(5..=30).contains(&cell_size) {
            return Err(ContractError::InvalidSettings("cell_size"));
        }
        let ramp = ramp.into();
        if ramp.is_empty()
            || ramp.len() > 64
            || !ramp.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
        {
            return Err(ContractError::InvalidRamp);
        }
        Ok(Self {
            cell_size,
            threshold,
            sampling,
            ramp,
        })
    }

    /// Returns the requested source-cell size.
    #[must_use]
    pub const fn cell_size(&self) -> u8 {
        self.cell_size
    }
    /// Returns the classic threshold value.
    #[must_use]
    pub const fn threshold(&self) -> u8 {
        self.threshold
    }
    /// Returns the sampling preset.
    #[must_use]
    pub const fn sampling(&self) -> Sampling {
        self.sampling
    }
    /// Returns the immutable ramp text.
    #[must_use]
    pub fn ramp(&self) -> &str {
        &self.ramp
    }
}
