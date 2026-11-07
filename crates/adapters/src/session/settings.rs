//! Preview and export settings, validated against the shared engine bounds.

use image_to_binary_core::{Sampling, MAX_CELL_SIZE, MAX_FONT_SIZE, MIN_CELL_SIZE, MIN_FONT_SIZE};

use super::error::SessionError;

/// Validated render settings shared by preview and export.
///
/// Invariants: the cell size is within `MIN_CELL_SIZE..=MAX_CELL_SIZE` and the font size
/// within `MIN_FONT_SIZE..=MAX_FONT_SIZE`. Every `u8` threshold is valid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreviewSettings {
    pub(super) cell_size: u8,
    pub(super) font_size: u8,
    pub(super) threshold: u8,
    pub(super) sampling: Sampling,
}

impl PreviewSettings {
    /// Validates the settings.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::SettingOutOfRange`] for a cell or font size outside its range.
    pub fn new(
        cell_size: u8,
        font_size: u8,
        threshold: u8,
        sampling: Sampling,
    ) -> Result<Self, SessionError> {
        if !(MIN_CELL_SIZE..=MAX_CELL_SIZE).contains(&cell_size) {
            return Err(SessionError::SettingOutOfRange {
                setting: "cellSize",
                min: MIN_CELL_SIZE,
                max: MAX_CELL_SIZE,
            });
        }
        if !(MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&font_size) {
            return Err(SessionError::SettingOutOfRange {
                setting: "fontSize",
                min: MIN_FONT_SIZE,
                max: MAX_FONT_SIZE,
            });
        }
        Ok(Self {
            cell_size,
            font_size,
            threshold,
            sampling,
        })
    }
}
