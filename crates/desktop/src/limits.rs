//! Resource ceilings for native processing and the engine bounds the UI reads.

use image_to_binary_adapters::session::{MAX_DECODED_BYTES as SHARED_DECODED, MAX_ENCODED_BYTES};
use image_to_binary_core::{
    ResourceLimits, MAX_CELL_SIZE, MAX_CONTRAST, MAX_FONT_SIZE, MAX_RAMP_LENGTH,
    MAX_SOURCE_CELL_HEIGHT, MAX_THRESHOLD, MIN_CELL_SIZE, MIN_CONTRAST, MIN_FONT_SIZE,
    MIN_THRESHOLD,
};
use serde::Serialize;

use crate::error::{ErrorCode, IpcError};

/// Encoded input ceiling shared with the browser session.
pub const MAX_ENCODED_INPUT_BYTES: u64 = MAX_ENCODED_BYTES;
/// Decoded RGBA working-set ceiling shared with the browser session.
pub const MAX_DECODED_BYTES: u64 = SHARED_DECODED;
/// Pixel and encoded-output ceilings shared with the core contract.
pub const NATIVE_RESOURCE_LIMITS: ResourceLimits = ResourceLimits::default();
/// Largest raw body accepted by the smoke echo command.
#[cfg(feature = "smoke")]
pub const SMOKE_ECHO_MAX_BYTES: usize = 64 * 1024;

/// Inclusive numeric range exposed to the UI.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Range<T> {
    min: T,
    max: T,
}

/// Native ceilings in bytes and pixels, all in the processing contract's units.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeLimits {
    pixel_limit: u64,
    encoded_input_limit_bytes: u64,
    decoded_limit_bytes: u64,
    output_limit_bytes: u64,
}

/// Shared engine bounds plus native ceilings returned by the `app_limits` command.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppLimits {
    cell_size: Range<u8>,
    font_size: Range<u8>,
    threshold: Range<u8>,
    contrast: Range<i8>,
    source_cell_height: Range<u16>,
    ramp_length: Range<usize>,
    native: NativeLimits,
}

/// Returns the bounds the native shell enforces, derived from the shared core constants.
#[must_use]
pub const fn app_limits() -> AppLimits {
    AppLimits {
        cell_size: Range {
            min: MIN_CELL_SIZE,
            max: MAX_CELL_SIZE,
        },
        font_size: Range {
            min: MIN_FONT_SIZE,
            max: MAX_FONT_SIZE,
        },
        threshold: Range {
            min: MIN_THRESHOLD,
            max: MAX_THRESHOLD,
        },
        contrast: Range {
            min: MIN_CONTRAST,
            max: MAX_CONTRAST,
        },
        source_cell_height: Range {
            min: 1,
            max: MAX_SOURCE_CELL_HEIGHT,
        },
        ramp_length: Range {
            min: 1,
            max: MAX_RAMP_LENGTH,
        },
        native: NativeLimits {
            pixel_limit: NATIVE_RESOURCE_LIMITS.max_pixels(),
            encoded_input_limit_bytes: MAX_ENCODED_INPUT_BYTES,
            decoded_limit_bytes: MAX_DECODED_BYTES,
            output_limit_bytes: NATIVE_RESOURCE_LIMITS.max_output_bytes(),
        },
    }
}

/// Rejects a declared payload length above `max` before any copy or decode buffer is sized.
///
/// # Errors
///
/// Returns [`ErrorCode::ResourceLimitExceeded`] when `length` is greater than `max`.
pub fn validate_payload_length(length: usize, max: usize) -> Result<(), IpcError> {
    if length > max {
        return Err(IpcError::new(
            ErrorCode::ResourceLimitExceeded,
            format!("payload of {length} bytes exceeds the {max} byte limit"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image_to_binary_core::ResourceLimits;
    use serde_json::json;

    #[test]
    fn shared_bounds_match_core_constants() {
        let limits = app_limits();
        assert_eq!(limits.cell_size.min, MIN_CELL_SIZE);
        assert_eq!(limits.cell_size.max, MAX_CELL_SIZE);
        assert_eq!(limits.font_size.min, MIN_FONT_SIZE);
        assert_eq!(limits.font_size.max, MAX_FONT_SIZE);
        assert_eq!(limits.threshold.min, MIN_THRESHOLD);
        assert_eq!(limits.threshold.max, MAX_THRESHOLD);
        assert_eq!(limits.contrast.min, MIN_CONTRAST);
        assert_eq!(limits.contrast.max, MAX_CONTRAST);
        assert_eq!(limits.source_cell_height.max, MAX_SOURCE_CELL_HEIGHT);
        assert_eq!(limits.ramp_length.max, MAX_RAMP_LENGTH);
    }

    #[test]
    fn native_limits_match_the_processing_contract() {
        let native = app_limits().native;
        assert_eq!(native.pixel_limit, 50_000_000);
        assert_eq!(native.pixel_limit, ResourceLimits::default().max_pixels());
        assert_eq!(native.output_limit_bytes, 256 * 1024 * 1024);
        assert_eq!(native.encoded_input_limit_bytes, 256 * 1024 * 1024);
        assert_eq!(native.decoded_limit_bytes, 1024 * 1024 * 1024);
    }

    #[test]
    fn ranges_are_ordered() {
        let limits = app_limits();
        assert!(limits.cell_size.min <= limits.cell_size.max);
        assert!(limits.font_size.min <= limits.font_size.max);
        assert!(limits.threshold.min <= limits.threshold.max);
        assert!(limits.contrast.min <= limits.contrast.max);
        assert!(limits.source_cell_height.min <= limits.source_cell_height.max);
        assert!(limits.ramp_length.min <= limits.ramp_length.max);
    }

    #[test]
    fn app_limits_serialize_with_camel_case_keys() {
        let json = serde_json::to_value(app_limits()).unwrap();
        assert_eq!(json["cellSize"], json!({ "min": 5, "max": 30 }));
        assert_eq!(json["rampLength"], json!({ "min": 1, "max": 64 }));
        assert_eq!(
            json["native"]["pixelLimit"],
            json!(50_000_000_u64),
            "native pixel ceiling must be exposed"
        );
        assert!(json["native"]["encodedInputLimitBytes"].is_u64());
        assert!(json["native"]["decodedLimitBytes"].is_u64());
        assert!(json["native"]["outputLimitBytes"].is_u64());
    }

    #[test]
    fn encoded_input_limit_is_the_shared_session_limit() {
        assert_eq!(MAX_ENCODED_INPUT_BYTES, 256 * 1024 * 1024);
        assert_eq!(MAX_DECODED_BYTES, 1024 * 1024 * 1024);
    }

    #[test]
    fn payload_length_is_rejected_before_it_is_copied() {
        assert!(validate_payload_length(64, 64).is_ok());
        let error = validate_payload_length(65, 64).unwrap_err();
        assert_eq!(error.code(), ErrorCode::ResourceLimitExceeded);
        assert!(error.message().contains("65"));
    }
}
