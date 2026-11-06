use crate::{ContractError, Ramp, MAX_CELL_SIZE, MIN_CELL_SIZE};

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
    /// Creates validated settings. Cell sizes use the shared cell-size bounds.
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
        if !(MIN_CELL_SIZE..=MAX_CELL_SIZE).contains(&cell_size) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MAX_CELL_SIZE, MAX_RAMP_LENGTH, MIN_CELL_SIZE};

    #[test]
    fn cell_size_accepts_shared_bounds_and_rejects_just_outside() {
        for cell_size in [MIN_CELL_SIZE, MAX_CELL_SIZE] {
            assert!(Settings::new(cell_size, 128, Sampling::Classic, "01").is_ok());
        }
        for cell_size in [MIN_CELL_SIZE - 1, MAX_CELL_SIZE + 1] {
            assert_eq!(
                Settings::new(cell_size, 128, Sampling::Classic, "01"),
                Err(ContractError::InvalidSettings("cell_size"))
            );
        }
    }

    #[test]
    fn threshold_accepts_every_u8_value_at_both_shared_bounds() {
        for threshold in [crate::MIN_THRESHOLD, crate::MAX_THRESHOLD] {
            let settings = Settings::new(5, threshold, Sampling::Classic, "01").unwrap();
            assert_eq!(settings.threshold(), threshold);
        }
    }

    #[test]
    fn ramp_accepts_sixty_four_entries_and_rejects_sixty_five() {
        let full = "a".repeat(MAX_RAMP_LENGTH);
        assert!(Settings::new(5, 128, Sampling::Classic, full.as_str()).is_ok());
        let oversized = "a".repeat(MAX_RAMP_LENGTH + 1);
        assert_eq!(
            Settings::new(5, 128, Sampling::Classic, oversized),
            Err(ContractError::InvalidRamp)
        );
        assert_eq!(
            ContractError::InvalidRamp.to_string(),
            "ramp must be printable ASCII and at most 64 characters"
        );
    }
}
