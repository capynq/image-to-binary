//! Primary-item HEIF/AVIF decoding through the safe libheif-rs wrapper.

use image::error::{DecodingError, ImageFormatHint};
use image::ImageError;
use image_to_binary_core::{ContractError, Dimensions};
use libheif_rs::{
    ColorSpace, HeifContext, HeifError, HeifErrorCode, HeifErrorSubCode, LibHeif, RgbChroma,
};

use super::{DecodeError, DecodeLimits, DecodedSource, NativeFormat};

/// Decodes the primary item into owned, tightly packed RGBA8.
///
/// Dimensions are checked against the pixel ceiling and decoded budget from
/// the image handle before any pixel is decoded. libheif applies `irot` and
/// `imir` transforms and produces 8-bit interleaved RGBA. The output is copied
/// row by row honouring the plane stride, and the output allocation is
/// fallible.
pub(super) fn decode_native(
    bytes: &[u8],
    format: NativeFormat,
    limits: DecodeLimits,
) -> Result<DecodedSource, DecodeError> {
    let lib = LibHeif::new_checked().map_err(|error| map_heif_error(&error, format))?;
    let mut context =
        HeifContext::read_from_bytes(bytes).map_err(|error| map_heif_error(&error, format))?;
    let mut security = context.security_limits();
    security.set_max_image_size_pixels(limits.resource.max_pixels());
    context
        .set_security_limits(&security)
        .map_err(|error| map_heif_error(&error, format))?;
    let handle = context
        .primary_image_handle()
        .map_err(|error| map_heif_error(&error, format))?;

    let handle_dimensions = Dimensions::new(handle.width(), handle.height())?;
    let output_len = limits.rgba_len(handle_dimensions)?;
    let working_len = u64::try_from(output_len)
        .ok()
        .and_then(|bytes| bytes.checked_mul(2))
        .ok_or(ContractError::ArithmeticOverflow)?;
    if working_len > limits.max_decoded_bytes {
        return Err(ContractError::ResourceLimitExceeded.into());
    }

    let image = lib
        .decode(&handle, ColorSpace::Rgb(RgbChroma::Rgba), None)
        .map_err(|error| map_heif_error(&error, format))?;
    let plane = image
        .planes()
        .interleaved
        .ok_or_else(|| malformed(format, "the primary item has no interleaved RGBA plane"))?;

    let dimensions = Dimensions::new(plane.width, plane.height)?;
    let output_len = limits.rgba_len(dimensions)?;
    let row_len = usize::try_from(dimensions.width())
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or(ContractError::ArithmeticOverflow)?;
    let last_row = usize::try_from(dimensions.height())
        .ok()
        .and_then(|height| height.checked_sub(1))
        .ok_or(ContractError::InvalidDimensions)?;
    let required_len = last_row
        .checked_mul(plane.stride)
        .and_then(|offset| offset.checked_add(row_len))
        .ok_or(ContractError::ArithmeticOverflow)?;
    if plane.stride < row_len || plane.data.len() < required_len {
        return Err(malformed(
            format,
            "the decoded RGBA plane is shorter than its declared stride",
        ));
    }

    let mut pixels = Vec::new();
    pixels.try_reserve_exact(output_len).map_err(|_| {
        DecodeError::ResourceLimit(format!(
            "the {format} output of {output_len} bytes could not be allocated"
        ))
    })?;
    for row in 0..=last_row {
        let start = row
            .checked_mul(plane.stride)
            .ok_or(ContractError::ArithmeticOverflow)?;
        let end = start
            .checked_add(row_len)
            .ok_or(ContractError::ArithmeticOverflow)?;
        let source = plane
            .data
            .get(start..end)
            .ok_or_else(|| malformed(format, "the decoded RGBA plane is truncated"))?;
        pixels.extend_from_slice(source);
    }
    DecodedSource::from_rgba(dimensions, pixels, limits).map_err(DecodeError::from)
}

/// Maps a libheif failure onto the processing error contract.
///
/// A missing decoder becomes `CodecUnavailable`. Security-limit and allocation
/// failures become `ResourceLimit`. Unsupported file types and features become
/// `UnsupportedFormat`. Every other failure, including truncated or corrupted
/// input, is a malformed-data `Image` error.
fn map_heif_error(error: &HeifError, format: NativeFormat) -> DecodeError {
    match (error.code, error.sub_code) {
        (HeifErrorCode::PluginLoadingError, HeifErrorSubCode::NoMatchingDecoderInstalled) => {
            DecodeError::CodecUnavailable(format)
        }
        (_, HeifErrorSubCode::SecurityLimitExceeded)
        | (HeifErrorCode::MemoryAllocationError, _) => {
            DecodeError::ResourceLimit(error.message.clone())
        }
        (HeifErrorCode::UnsupportedFileType | HeifErrorCode::UnsupportedFeature, _) => {
            DecodeError::UnsupportedFormat
        }
        _ => malformed(format, &error.message),
    }
}

fn malformed(format: NativeFormat, detail: &str) -> DecodeError {
    DecodeError::Image(ImageError::Decoding(DecodingError::new(
        ImageFormatHint::Name(format.to_string()),
        detail.to_owned(),
    )))
}

#[cfg(test)]
mod tests {
    use image_to_binary_core::ResourceLimits;

    use super::*;

    const HEIC: &[u8] = include_bytes!("../../tests/fixtures/heif/sample.heic");
    const AVIF: &[u8] = include_bytes!("../../tests/fixtures/heif/sample.avif");

    fn assert_near(actual: &[u8], expected: [u8; 4], tolerance: u8) {
        for (channel, (&actual, &expected)) in actual.iter().zip(expected.iter()).enumerate() {
            assert!(
                actual.abs_diff(expected) <= tolerance,
                "channel {channel}: {actual} is not within {tolerance} of {expected}"
            );
        }
    }

    #[test]
    fn heic_and_avif_fixtures_decode_primary_items() {
        for (bytes, expected) in [(HEIC, [0, 2, 121, 255]), (AVIF, [1, 0, 129, 255])] {
            let decoded = super::super::decode(bytes, DecodeLimits::default()).unwrap();
            assert_eq!(
                decoded.metadata().dimensions(),
                Dimensions::new(64, 48).unwrap()
            );
            assert_eq!(decoded.pixels().len(), 12_288);
            assert_near(&decoded.pixels()[..4], expected, 12);
        }
    }

    #[test]
    fn decoding_twice_is_byte_stable() {
        for bytes in [HEIC, AVIF] {
            let first = super::super::decode(bytes, DecodeLimits::default()).unwrap();
            let second = super::super::decode(bytes, DecodeLimits::default()).unwrap();
            assert_eq!(first, second);
        }
    }

    #[test]
    fn pixel_limit_rejects_before_pixel_decode() {
        let limits = DecodeLimits::new(
            ResourceLimits::new(64 * 48 - 1, 256 * 1024 * 1024).unwrap(),
            256 * 1024 * 1024,
            1024 * 1024 * 1024,
        )
        .unwrap();
        for bytes in [HEIC, AVIF] {
            assert!(matches!(
                super::super::decode(bytes, limits),
                Err(DecodeError::Contract(ContractError::ResourceLimitExceeded))
            ));
        }
    }

    #[test]
    fn decoded_budget_counts_libheif_output_and_copy() {
        let limits = DecodeLimits::new(
            ResourceLimits::default(),
            256 * 1024 * 1024,
            64 * 48 * 4 * 2 - 1,
        )
        .unwrap();
        assert!(matches!(
            super::super::decode(HEIC, limits),
            Err(DecodeError::Contract(ContractError::ResourceLimitExceeded))
        ));
    }

    #[test]
    fn truncated_and_corrupted_inputs_return_typed_errors() {
        let truncated = &HEIC[..HEIC.len() / 2];
        let mut corrupted = HEIC.to_vec();
        corrupted[396..426].fill(0xff);
        let mut unrecognized = AVIF.to_vec();
        unrecognized[4..8].copy_from_slice(b"xxxx");
        for bytes in [truncated, corrupted.as_slice(), unrecognized.as_slice()] {
            let result = super::super::decode(bytes, DecodeLimits::default());
            assert!(
                matches!(
                    result,
                    Err(DecodeError::Image(_)
                        | DecodeError::ResourceLimit(_)
                        | DecodeError::UnsupportedFormat)
                ),
                "unexpected result: {result:?}"
            );
        }
    }

    #[test]
    fn missing_decoder_maps_to_codec_unavailable() {
        let error = HeifError {
            code: HeifErrorCode::PluginLoadingError,
            sub_code: HeifErrorSubCode::NoMatchingDecoderInstalled,
            message: String::from("no decoder"),
        };
        assert!(matches!(
            map_heif_error(&error, NativeFormat::Avif),
            DecodeError::CodecUnavailable(NativeFormat::Avif)
        ));
    }

    #[test]
    fn security_and_allocation_failures_map_to_resource_limits() {
        let security = HeifError {
            code: HeifErrorCode::InvalidInput,
            sub_code: HeifErrorSubCode::SecurityLimitExceeded,
            message: String::from("too large"),
        };
        let allocation = HeifError {
            code: HeifErrorCode::MemoryAllocationError,
            sub_code: HeifErrorSubCode::Unspecified,
            message: String::from("no memory"),
        };
        for error in [security, allocation] {
            assert!(matches!(
                map_heif_error(&error, NativeFormat::Heif),
                DecodeError::ResourceLimit(_)
            ));
        }
    }
}
