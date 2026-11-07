//! Validated source geometry and encoded input, checked before any pixel or byte is copied.

use image_to_binary_core::{Dimensions, ResourceLimits};
use image_to_binary_processing::{decode, DecodeLimits, DecodedSource};

use super::error::SessionError;
use super::limits::{MAX_DECODED_BYTES, MAX_ENCODED_BYTES};
use super::Session;

/// Validated source geometry. Every check runs before the caller allocates pixel memory.
///
/// Invariants: the dimensions fit the pixel ceiling and the decode limits, and the
/// expected RGBA length is `width * height * 4` for those dimensions.
#[derive(Clone, Copy, Debug)]
pub struct SourcePlan {
    dimensions: Dimensions,
    limits: DecodeLimits,
    len: usize,
}

impl SourcePlan {
    /// Validates the geometry. A `max_pixels` ceiling replaces the default 50 MP limit.
    ///
    /// # Errors
    ///
    /// Returns a contract error for a zero or oversized ceiling, `DimensionsTooLarge`
    /// when a side exceeds `u32`, and a contract error for zero or exceeded dimensions.
    pub fn new(width: u64, height: u64, max_pixels: Option<u64>) -> Result<Self, SessionError> {
        let limits = match max_pixels {
            None => DecodeLimits::default(),
            Some(ceiling) => {
                let resource =
                    ResourceLimits::new(ceiling, ResourceLimits::default().max_output_bytes())?;
                DecodeLimits::new(resource, MAX_ENCODED_BYTES, MAX_DECODED_BYTES)?
            }
        };
        let dimensions = Dimensions::new(
            u32::try_from(width).map_err(|_| SessionError::DimensionsTooLarge)?,
            u32::try_from(height).map_err(|_| SessionError::DimensionsTooLarge)?,
        )?;
        let len = limits.rgba_len(dimensions)?;
        Ok(Self {
            dimensions,
            limits,
            len,
        })
    }

    /// Rejects a pixel buffer whose length is not the planned RGBA length.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::SourceLength`] when `actual` differs from the plan.
    pub fn check_len(&self, actual: usize) -> Result<(), SessionError> {
        if actual == self.len {
            Ok(())
        } else {
            Err(SessionError::SourceLength)
        }
    }

    /// Reserves a zeroed pixel buffer of the planned length with a fallible allocation.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::SourceReservation`] when the allocator refuses the request.
    pub fn allocate(&self) -> Result<Vec<u8>, SessionError> {
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(self.len)
            .map_err(|_| SessionError::SourceReservation)?;
        buffer.resize(self.len, 0);
        Ok(buffer)
    }

    /// Moves `pixels` into a new session without cloning them.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`DecodedSource::from_rgba`] and [`Session::new`].
    pub fn into_session(self, pixels: Vec<u8>) -> Result<Session, SessionError> {
        Session::new(DecodedSource::from_rgba(
            self.dimensions,
            pixels,
            self.limits,
        )?)
    }
}

pub(super) fn reserve_zeroed(len: usize) -> Result<Vec<u8>, SessionError> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(len)
        .map_err(|_| SessionError::EncodedReservation)?;
    buffer.resize(len, 0);
    Ok(buffer)
}

/// Validated encoded-input length. The 256 MiB ceiling is checked before any copy.
#[derive(Clone, Copy, Debug)]
pub struct EncodedPlan {
    pub(super) len: usize,
}

impl EncodedPlan {
    /// Accepts a length at most [`MAX_ENCODED_BYTES`].
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::EncodedTooLarge`] above the ceiling.
    pub fn new(len: usize) -> Result<Self, SessionError> {
        if u64::try_from(len).unwrap_or(u64::MAX) > MAX_ENCODED_BYTES {
            return Err(SessionError::EncodedTooLarge);
        }
        Ok(Self { len })
    }

    /// Reserves a zeroed destination of the planned length with a fallible allocation.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::EncodedReservation`] when the allocator refuses the request.
    pub fn allocate(self) -> Result<Vec<u8>, SessionError> {
        reserve_zeroed(self.len)
    }

    /// Decodes `encoded` once with the default limits and builds a session from the
    /// normalized pixels. The encoded buffer is dropped before the session is returned,
    /// and decoded pixels are moved, not cloned.
    ///
    /// # Errors
    ///
    /// Returns the decoder's failure, reduced to a stable code, and builds no session on
    /// error. Decoder allocations are best effort.
    pub fn decode_session(encoded: Vec<u8>) -> Result<Session, SessionError> {
        let source = decode(&encoded, DecodeLimits::default())?;
        drop(encoded);
        Session::new(source)
    }
}
