//! Typed IPC errors that reuse the stable machine-readable codes of the WASM boundary.

use std::fmt;

use image_to_binary_adapters::PngTaskError;
use image_to_binary_core::ContractError;
use image_to_binary_export::ExportError;
use image_to_binary_processing::{DecodeError, RenderError};
use serde::Serialize;

/// Stable error codes; the strings match `crates/wasm/src/error.rs` one for one.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidArgument,
    InvalidDimensions,
    InvalidSettings,
    InvalidProgress,
    ResourceLimitExceeded,
    ArithmeticOverflow,
    Cancelled,
    StaleGeneration,
    Disposed,
    NoJob,
    JobIncomplete,
    FrameConsumed,
    EncodedTooLarge,
    UnsupportedFormat,
    CodecUnavailable,
    DecodeFailed,
    ReservationFailed,
    EncodeFailed,
    Internal,
}

impl ErrorCode {
    /// Returns the wire string shared with the WASM error vocabulary.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArgument => "invalid_argument",
            Self::InvalidDimensions => "invalid_dimensions",
            Self::InvalidSettings => "invalid_settings",
            Self::InvalidProgress => "invalid_progress",
            Self::ResourceLimitExceeded => "resource_limit_exceeded",
            Self::ArithmeticOverflow => "arithmetic_overflow",
            Self::Cancelled => "cancelled",
            Self::StaleGeneration => "stale_generation",
            Self::Disposed => "disposed",
            Self::NoJob => "no_job",
            Self::JobIncomplete => "job_incomplete",
            Self::FrameConsumed => "frame_consumed",
            Self::EncodedTooLarge => "encoded_too_large",
            Self::UnsupportedFormat => "unsupported_format",
            Self::CodecUnavailable => "codec_unavailable",
            Self::DecodeFailed => "decode_failed",
            Self::ReservationFailed => "reservation_failed",
            Self::EncodeFailed => "encode_failed",
            Self::Internal => "internal",
        }
    }
}

/// Error returned by every fallible IPC command, serialised as `{ code, message }`.
///
/// Invariant: `code` is one of [`ErrorCode`], so the frontend switches on a closed set.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct IpcError {
    code: ErrorCode,
    message: String,
}

impl IpcError {
    /// Creates an error with a stable code and a human-readable message.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Returns the stable error code.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        self.code
    }

    /// Returns the diagnostic message; it may contain engine detail but never payload bytes.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for IpcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for IpcError {}

impl From<ContractError> for IpcError {
    fn from(error: ContractError) -> Self {
        let code = match error {
            ContractError::InvalidDimensions => ErrorCode::InvalidDimensions,
            ContractError::ResourceLimitExceeded => ErrorCode::ResourceLimitExceeded,
            ContractError::ArithmeticOverflow => ErrorCode::ArithmeticOverflow,
            ContractError::InvalidSettings(_) | ContractError::InvalidRamp => {
                ErrorCode::InvalidSettings
            }
            ContractError::InvalidProgress => ErrorCode::InvalidProgress,
            ContractError::Cancelled => ErrorCode::Cancelled,
            ContractError::StaleGeneration => ErrorCode::StaleGeneration,
        };
        Self::new(code, error.to_string())
    }
}

impl From<RenderError> for IpcError {
    fn from(error: RenderError) -> Self {
        match error {
            RenderError::Contract(inner) => Self::from(inner),
            RenderError::InvalidFont(message) => Self::new(
                ErrorCode::Internal,
                format!("bundled font is invalid: {message}"),
            ),
        }
    }
}

impl From<DecodeError> for IpcError {
    fn from(error: DecodeError) -> Self {
        match error {
            DecodeError::EncodedInputTooLarge => Self::new(
                ErrorCode::EncodedTooLarge,
                "encoded input exceeds the 256 MiB limit",
            ),
            DecodeError::UnsupportedFormat => Self::new(
                ErrorCode::UnsupportedFormat,
                "the image format is not supported",
            ),
            DecodeError::CodecUnavailable(format) => Self::new(
                ErrorCode::CodecUnavailable,
                format!("{format} decoding is not available in this build"),
            ),
            DecodeError::Contract(inner) => Self::from(inner),
            DecodeError::ResourceLimit(diagnostic) => Self::new(
                ErrorCode::ResourceLimitExceeded,
                format!("the image exceeds a decoder resource limit: {diagnostic}"),
            ),
            DecodeError::Image(inner) => Self::new(
                ErrorCode::DecodeFailed,
                format!("the image data could not be decoded: {inner}"),
            ),
            DecodeError::Io(inner) => Self::new(
                ErrorCode::DecodeFailed,
                format!("the image data could not be read: {inner}"),
            ),
        }
    }
}

impl From<PngTaskError> for IpcError {
    fn from(error: PngTaskError) -> Self {
        match error {
            PngTaskError::Contract(inner) => Self::from(inner),
            PngTaskError::Render(inner) => Self::from(inner),
            PngTaskError::Export(inner) => Self::from(inner),
            PngTaskError::Terminated => {
                Self::new(ErrorCode::Internal, "the PNG export was already terminated")
            }
        }
    }
}

impl From<ExportError> for IpcError {
    fn from(error: ExportError) -> Self {
        match error {
            ExportError::Contract(inner) => Self::from(inner),
            ExportError::Row(inner) => Self::from(*inner),
            ExportError::Png(inner) => Self::new(
                ErrorCode::EncodeFailed,
                format!("the PNG encoder failed: {inner}"),
            ),
            ExportError::Sink(reason) => Self::new(
                ErrorCode::Internal,
                format!("PNG chunk storage failed: {reason}"),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image_to_binary_processing::NativeFormat;

    const ALL_CODES: [ErrorCode; 19] = [
        ErrorCode::InvalidArgument,
        ErrorCode::InvalidDimensions,
        ErrorCode::InvalidSettings,
        ErrorCode::InvalidProgress,
        ErrorCode::ResourceLimitExceeded,
        ErrorCode::ArithmeticOverflow,
        ErrorCode::Cancelled,
        ErrorCode::StaleGeneration,
        ErrorCode::Disposed,
        ErrorCode::NoJob,
        ErrorCode::JobIncomplete,
        ErrorCode::FrameConsumed,
        ErrorCode::EncodedTooLarge,
        ErrorCode::UnsupportedFormat,
        ErrorCode::CodecUnavailable,
        ErrorCode::DecodeFailed,
        ErrorCode::ReservationFailed,
        ErrorCode::EncodeFailed,
        ErrorCode::Internal,
    ];

    #[test]
    fn serialized_codes_match_the_wire_strings() {
        for code in ALL_CODES {
            let json = serde_json::to_value(code).unwrap();
            assert_eq!(json, serde_json::Value::from(code.as_str()));
        }
    }

    #[test]
    fn wire_strings_are_unique_snake_case() {
        let mut seen = std::collections::BTreeSet::new();
        for code in ALL_CODES {
            let text = code.as_str();
            assert!(text.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
            assert!(seen.insert(text), "duplicate code {text}");
        }
        assert_eq!(seen.len(), ALL_CODES.len());
    }

    #[test]
    fn ipc_error_serializes_code_and_message_only() {
        let error = IpcError::new(ErrorCode::ResourceLimitExceeded, "too big");
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "code": "resource_limit_exceeded", "message": "too big" })
        );
    }

    #[test]
    fn contract_errors_map_to_stable_codes() {
        let cases = [
            (ContractError::InvalidDimensions, "invalid_dimensions"),
            (
                ContractError::ResourceLimitExceeded,
                "resource_limit_exceeded",
            ),
            (ContractError::ArithmeticOverflow, "arithmetic_overflow"),
            (ContractError::InvalidSettings("x"), "invalid_settings"),
            (ContractError::InvalidRamp, "invalid_settings"),
            (ContractError::InvalidProgress, "invalid_progress"),
            (ContractError::Cancelled, "cancelled"),
            (ContractError::StaleGeneration, "stale_generation"),
        ];
        for (error, expected) in cases {
            assert_eq!(IpcError::from(error).code().as_str(), expected);
        }
    }

    #[test]
    fn export_failures_map_to_stable_codes() {
        let cases = [
            (
                PngTaskError::Contract(ContractError::ResourceLimitExceeded),
                "resource_limit_exceeded",
            ),
            (
                PngTaskError::Contract(ContractError::Cancelled),
                "cancelled",
            ),
            (
                PngTaskError::Contract(ContractError::StaleGeneration),
                "stale_generation",
            ),
            (
                PngTaskError::Export(ExportError::Row(Box::new(ExportError::Contract(
                    ContractError::Cancelled,
                )))),
                "cancelled",
            ),
            (
                PngTaskError::Export(ExportError::Sink("poisoned")),
                "internal",
            ),
            (PngTaskError::Terminated, "internal"),
        ];
        for (error, expected) in cases {
            assert_eq!(IpcError::from(error).code().as_str(), expected);
        }
    }

    #[test]
    fn decode_failures_map_to_stable_codes() {
        let cases = [
            (DecodeError::EncodedInputTooLarge, "encoded_too_large"),
            (DecodeError::UnsupportedFormat, "unsupported_format"),
            (
                DecodeError::CodecUnavailable(NativeFormat::Heif),
                "codec_unavailable",
            ),
            (
                DecodeError::Contract(ContractError::InvalidDimensions),
                "invalid_dimensions",
            ),
            (
                DecodeError::ResourceLimit(String::from("Memory limit exceeded")),
                "resource_limit_exceeded",
            ),
            (
                DecodeError::Io(std::io::Error::other("read failed")),
                "decode_failed",
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(IpcError::from(error).code().as_str(), expected);
        }
    }
}
