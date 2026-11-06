use crate::{ContractError, Ramp};

/// Sampling behavior; codecs and rendering are implemented in later phases.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Sampling {
    Classic,
    Average,
}

/// Output glyph selection for one rendering pass.
///
/// `Ascii` entries are ordered from least ink to most ink. `Block` uses the
/// five block-density levels from space to full block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderMode {
    Binary,
    Ascii(Ramp),
    Block,
}

/// Deterministic settings shared by preview and export.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Settings {
    cell_size: u8,
    threshold: u8,
    sampling: Sampling,
    ramp: Ramp,
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
        let text: String = ramp.into();
        let ramp = Ramp::new(&text)?;
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
        self.ramp.as_str()
    }
}
