//! Binary layout of a finished preview frame. It travels as a raw IPC body, never as JSON or
//! base64, so the page reads it with a single `ArrayBuffer` view.
//!
//! Layout, every integer little-endian:
//!
//! | offset | size | field                                              |
//! |--------|------|----------------------------------------------------|
//! | 0      | 4    | magic `I2BF`                                       |
//! | 4      | 4    | version, `u32`, currently `1`                      |
//! | 8      | 8    | job generation, `u64`                              |
//! | 16     | 4    | width in pixels, `u32`                             |
//! | 20     | 4    | height in pixels, `u32`                            |
//! | 24     | n    | RGBA8 pixels, row-major, `n = width * height * 4`  |
//!
//! Invariants: the body is exactly `24 + n` bytes, and `n` never exceeds
//! [`PREVIEW_FRAME_MAX_BYTES`], the renderer's output ceiling. Encoding rejects any other
//! pixel length or size before a byte is written.

use crate::error::{ErrorCode, IpcError};
use crate::limits::NATIVE_RESOURCE_LIMITS;

/// Magic bytes at the start of every frame body.
pub(crate) const FRAME_MAGIC: [u8; 4] = *b"I2BF";
/// Layout version. A reader rejects any other value.
pub(crate) const FRAME_VERSION: u32 = 1;
/// Bytes preceding the RGBA pixels.
pub(crate) const FRAME_HEADER_BYTES: usize = 24;
/// Largest RGBA payload accepted for one preview: the renderer's output ceiling.
pub(crate) const PREVIEW_FRAME_MAX_BYTES: u64 = NATIVE_RESOURCE_LIMITS.max_output_bytes();

/// Serialises a preview frame, bounded by [`PREVIEW_FRAME_MAX_BYTES`].
///
/// # Errors
///
/// Returns `invalid_dimensions` when `pixels` is not exactly `width * height * 4` bytes,
/// `arithmetic_overflow` when the size cannot be represented, and `resource_limit_exceeded`
/// when the payload exceeds the ceiling.
pub(crate) fn encode_frame(
    generation: u64,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Result<Vec<u8>, IpcError> {
    encode_within(PREVIEW_FRAME_MAX_BYTES, generation, width, height, pixels)
}

fn encode_within(
    cap: u64,
    generation: u64,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Result<Vec<u8>, IpcError> {
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|count| count.checked_mul(4))
        .ok_or_else(|| {
            IpcError::new(
                ErrorCode::ArithmeticOverflow,
                "the frame size cannot be represented",
            )
        })?;
    if u64::try_from(pixels.len()).ok() != Some(expected) {
        return Err(IpcError::new(
            ErrorCode::InvalidDimensions,
            "frame pixels must contain exactly width * height * 4 bytes",
        ));
    }
    if expected > cap {
        return Err(IpcError::new(
            ErrorCode::ResourceLimitExceeded,
            format!("the preview frame exceeds the {cap} byte limit"),
        ));
    }
    let mut body = Vec::with_capacity(FRAME_HEADER_BYTES + pixels.len());
    body.extend_from_slice(&FRAME_MAGIC);
    body.extend_from_slice(&FRAME_VERSION.to_le_bytes());
    body.extend_from_slice(&generation.to_le_bytes());
    body.extend_from_slice(&width.to_le_bytes());
    body.extend_from_slice(&height.to_le_bytes());
    body.extend_from_slice(pixels);
    Ok(body)
}

/// A frame read back from its body. Used by tests to check the layout the page parses.
#[cfg(test)]
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct DecodedFrame<'body> {
    pub(crate) generation: u64,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pixels: &'body [u8],
}

/// Parses a frame body and checks the magic, version and exact length.
///
/// # Errors
///
/// Returns `invalid_argument` for a short, mis-tagged or mis-sized body.
#[cfg(test)]
pub(crate) fn decode_frame(body: &[u8]) -> Result<DecodedFrame<'_>, IpcError> {
    let malformed = || IpcError::new(ErrorCode::InvalidArgument, "the frame body is malformed");
    let header = body.get(..FRAME_HEADER_BYTES).ok_or_else(malformed)?;
    if header.get(..4) != Some(FRAME_MAGIC.as_slice()) || le_u32(header, 4) != Some(FRAME_VERSION) {
        return Err(malformed());
    }
    let generation = le_u64(header, 8).ok_or_else(malformed)?;
    let width = le_u32(header, 16).ok_or_else(malformed)?;
    let height = le_u32(header, 20).ok_or_else(malformed)?;
    let pixels = body.get(FRAME_HEADER_BYTES..).ok_or_else(malformed)?;
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|count| count.checked_mul(4))
        .ok_or_else(malformed)?;
    if u64::try_from(pixels.len()).ok() != Some(expected) {
        return Err(malformed());
    }
    Ok(DecodedFrame {
        generation,
        width,
        height,
        pixels,
    })
}

#[cfg(test)]
fn le_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(test)]
fn le_u64(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(width: u32, height: u32) -> Vec<u8> {
        (0..width * height * 4)
            .map(|index| u8::try_from(index % 251).unwrap())
            .collect()
    }

    #[test]
    fn frame_header_round_trips_generation_size_and_pixels() {
        let pixels = sample(3, 2);
        let body = encode_frame(u64::MAX - 7, 3, 2, &pixels).unwrap();
        assert_eq!(body.len(), FRAME_HEADER_BYTES + pixels.len());
        let decoded = decode_frame(&body).unwrap();
        assert_eq!(decoded.generation, u64::MAX - 7);
        assert_eq!((decoded.width, decoded.height), (3, 2));
        assert_eq!(decoded.pixels, pixels.as_slice());
    }

    #[test]
    fn frame_header_is_fixed_little_endian() {
        let body = encode_frame(0x0102_0304_0506_0708, 1, 1, &[9, 8, 7, 6]).unwrap();
        assert_eq!(&body[..4], b"I2BF");
        assert_eq!(&body[4..8], &1_u32.to_le_bytes());
        assert_eq!(&body[8..16], &0x0102_0304_0506_0708_u64.to_le_bytes());
        assert_eq!(&body[16..20], &1_u32.to_le_bytes());
        assert_eq!(&body[20..24], &1_u32.to_le_bytes());
        assert_eq!(&body[24..], &[9, 8, 7, 6]);
    }

    #[test]
    fn encoding_rejects_a_mismatched_pixel_length() {
        let error = encode_frame(1, 2, 2, &[0; 15]).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidDimensions);
        assert_eq!(
            encode_frame(1, 2, 2, &[0; 17]).unwrap_err().code(),
            ErrorCode::InvalidDimensions
        );
    }

    #[test]
    fn encoding_is_bounded_by_the_preview_cap() {
        let pixels = sample(2, 2);
        assert!(encode_within(16, 1, 2, 2, &pixels).is_ok());
        assert_eq!(
            encode_within(15, 1, 2, 2, &pixels).unwrap_err().code(),
            ErrorCode::ResourceLimitExceeded
        );
        assert_eq!(PREVIEW_FRAME_MAX_BYTES, 256 * 1024 * 1024);
        assert_eq!(
            encode_frame(1, u32::MAX, u32::MAX, &[]).unwrap_err().code(),
            ErrorCode::ArithmeticOverflow
        );
    }

    #[test]
    fn decoding_rejects_bad_magic_version_and_length() {
        let body = encode_frame(4, 1, 1, &[1, 2, 3, 4]).unwrap();
        let mut bad_magic = body.clone();
        bad_magic[0] ^= 0xff;
        assert!(decode_frame(&bad_magic).is_err());
        let mut bad_version = body.clone();
        bad_version[4] = 2;
        assert!(decode_frame(&bad_version).is_err());
        assert!(decode_frame(&body[..body.len() - 1]).is_err());
        assert!(decode_frame(&body[..10]).is_err());
        let mut extra = body.clone();
        extra.push(0);
        assert!(decode_frame(&extra).is_err());
    }
}
