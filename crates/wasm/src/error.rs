//! Stable machine-readable error codes shared by the session and the JS boundary.

use std::fmt;

use image_to_binary_adapters::PngTaskError;
use image_to_binary_core::ContractError;
use image_to_binary_export::ExportError;
use image_to_binary_processing::{DecodeError, RenderError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ErrorCode {
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
    pub(crate) const fn as_str(self) -> &'static str {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BindingError {
    code: ErrorCode,
    message: String,
}

impl BindingError {
    pub(crate) fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) const fn code(&self) -> ErrorCode {
        self.code
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for BindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code.as_str(), self.message)
    }
}

impl From<ContractError> for BindingError {
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

impl From<RenderError> for BindingError {
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

impl From<DecodeError> for BindingError {
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

/// Maps a PNG export failure to a stable code. Resource ceilings (output bytes,
/// pending storage, a row that cannot fit the batch ceiling) map to
/// `resource_limit_exceeded`; cancellation and generation breaches keep their
/// contract codes. A PNG encoder failure is `encode_failed`. A poisoned chunk
/// store or an already-terminated task is `internal`, because the session never
/// reaches them through its public calls unless a bug or panic occurred earlier.
impl From<PngTaskError> for BindingError {
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

impl From<ExportError> for BindingError {
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
    use image_to_binary_processing::{decode, DecodeLimits, NativeFormat};

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
            assert_eq!(BindingError::from(error).code().as_str(), expected);
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
                PngTaskError::Contract(ContractError::InvalidProgress),
                "invalid_progress",
            ),
            (
                PngTaskError::Export(ExportError::Contract(ContractError::ResourceLimitExceeded)),
                "resource_limit_exceeded",
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
            assert_eq!(BindingError::from(error).code().as_str(), expected);
        }
    }

    #[test]
    fn decode_errors_map_to_stable_codes() {
        let malformed =
            decode(&[137, 80, 78, 71, 13, 10, 26, 10], DecodeLimits::default()).unwrap_err();
        assert!(matches!(malformed, DecodeError::Image(_)));
        let cases = [
            (DecodeError::EncodedInputTooLarge, "encoded_too_large"),
            (DecodeError::UnsupportedFormat, "unsupported_format"),
            (
                DecodeError::CodecUnavailable(NativeFormat::Heif),
                "codec_unavailable",
            ),
            (
                DecodeError::Contract(ContractError::ResourceLimitExceeded),
                "resource_limit_exceeded",
            ),
            (
                DecodeError::Contract(ContractError::InvalidDimensions),
                "invalid_dimensions",
            ),
            (
                DecodeError::Io(std::io::Error::other("read failed")),
                "decode_failed",
            ),
            (
                DecodeError::ResourceLimit(String::from("Memory limit exceeded")),
                "resource_limit_exceeded",
            ),
            (malformed, "decode_failed"),
        ];
        for (error, expected) in cases {
            assert_eq!(BindingError::from(error).code().as_str(), expected);
        }
    }
}
