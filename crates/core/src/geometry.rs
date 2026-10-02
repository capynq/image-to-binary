use crate::ContractError;

/// Positive image dimensions with checked area calculation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Dimensions {
    width: u32,
    height: u32,
}

impl Dimensions {
    /// Creates dimensions and rejects zero values or multiplication overflow.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidDimensions`] for a zero dimension.
    pub fn new(width: u32, height: u32) -> Result<Self, ContractError> {
        if width == 0 || height == 0 || u64::from(width).checked_mul(u64::from(height)).is_none() {
            return Err(ContractError::InvalidDimensions);
        }
        Ok(Self { width, height })
    }

    /// Returns the width in source pixels.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Returns the height in source pixels.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    /// Returns the pixel count using checked arithmetic.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::ArithmeticOverflow`] if multiplication cannot
    /// be represented.
    pub fn pixel_count(self) -> Result<u64, ContractError> {
        u64::from(self.width)
            .checked_mul(u64::from(self.height))
            .ok_or(ContractError::ArithmeticOverflow)
    }
}

/// Allocation and output ceilings owned by the processing layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceLimits {
    max_pixels: u64,
    max_output_bytes: u64,
}

impl ResourceLimits {
    /// The default 50-megapixel input ceiling from the product contract.
    #[must_use]
    pub const fn default() -> Self {
        Self {
            max_pixels: 50_000_000,
            max_output_bytes: 256 * 1024 * 1024,
        }
    }

    /// Creates explicit positive limits.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidSettings`] for zero values or a pixel
    /// ceiling above the product's 50-megapixel hard limit.
    pub const fn new(max_pixels: u64, max_output_bytes: u64) -> Result<Self, ContractError> {
        if max_pixels == 0 || max_output_bytes == 0 || max_pixels > 50_000_000 {
            return Err(ContractError::InvalidSettings("resource limits"));
        }
        Ok(Self {
            max_pixels,
            max_output_bytes,
        })
    }

    /// Validates dimensions before a decoder or renderer allocates storage.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::ResourceLimitExceeded`] above the configured
    /// pixel ceiling.
    pub fn validate(self, dimensions: Dimensions) -> Result<(), ContractError> {
        if dimensions.pixel_count()? > self.max_pixels {
            Err(ContractError::ResourceLimitExceeded)
        } else {
            Ok(())
        }
    }

    /// Returns the input pixel ceiling.
    #[must_use]
    pub const fn max_pixels(self) -> u64 {
        self.max_pixels
    }

    /// Returns the encoded output ceiling.
    #[must_use]
    pub const fn max_output_bytes(self) -> u64 {
        self.max_output_bytes
    }
}
