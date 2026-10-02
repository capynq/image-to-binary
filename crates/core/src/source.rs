use crate::{ContractError, Dimensions};

/// Metadata for a decoded source, before pixel storage is allocated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceMetadata {
    dimensions: Dimensions,
    channels: u8,
}

impl SourceMetadata {
    /// Creates metadata for 1, 2, 3, or 4-channel normalized input.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidSettings`] for an unsupported channel
    /// count.
    pub fn new(dimensions: Dimensions, channels: u8) -> Result<Self, ContractError> {
        if !(1..=4).contains(&channels) {
            return Err(ContractError::InvalidSettings("channels"));
        }
        Ok(Self {
            dimensions,
            channels,
        })
    }
    /// Returns source dimensions.
    #[must_use]
    pub const fn dimensions(self) -> Dimensions {
        self.dimensions
    }
    /// Returns normalized channel count.
    #[must_use]
    pub const fn channels(self) -> u8 {
        self.channels
    }
}
