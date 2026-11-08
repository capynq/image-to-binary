//! Transport-neutral session failures and the stable codes every front end exposes.

use std::fmt;

use image_to_binary_core::ContractError;
use image_to_binary_export::ExportError;
use image_to_binary_processing::{DecodeError, RenderError};

use super::limits::MAX_ENCODED_BYTES;
use crate::PngTaskError;

/// Stable machine-readable failure codes. The wire strings are shared by the WASM
/// and native shells and must not change; each variant maps to exactly one string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorCode {
    /// A JS or IPC argument has the wrong type, is not an integer, or is outside a safe range.
    InvalidArgument,
    /// Image dimensions or a pixel buffer length are zero or inconsistent.
    InvalidDimensions,
    /// A render setting or limit is outside its allowed range or vocabulary.
    InvalidSettings,
    /// A progress unit is zero, overflows, or exceeds its total.
    InvalidProgress,
    /// A pixel, byte, or allocation ceiling was exceeded.
    ResourceLimitExceeded,
    /// A size or count could not be represented.
    ArithmeticOverflow,
    /// The job was cancelled or its worker shut down.
    Cancelled,
    /// The generation is not the current job or does not exceed earlier ones.
    StaleGeneration,
    /// The session was released; no further calls succeed.
    Disposed,
    /// No job, source, or job of the requested kind exists for the generation.
    NoJob,
    /// A result was requested before the job finished its stage.
    JobIncomplete,
    /// The preview frame's pixels were already taken.
    FrameConsumed,
    /// The encoded input exceeds the 256 MiB limit.
    EncodedTooLarge,
    /// The bytes are not in a supported image format.
    UnsupportedFormat,
    /// The format is recognised but its codec is not compiled in.
    CodecUnavailable,
    /// The bytes are malformed or could not be read.
    DecodeFailed,
    /// The allocator refused a fallible reservation.
    ReservationFailed,
    /// The PNG encoder reported a failure.
    EncodeFailed,
    /// An invariant broke or a sink failed; not caused by the input.
    Internal,
}

impl ErrorCode {
    /// Returns the wire string, which is identical in every front end.
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

/// Which kind of job a usage error refers to, so each kind keeps its own message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobKind {
    /// A preview frame, original or artwork.
    Preview,
    /// A PNG export streamed into a sink.
    Export,
}

/// Why a generation was refused. `NotCurrent` names a call for a job other than the
/// newest one; `NotNewer` names a start that does not exceed every accepted generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StaleReason {
    /// The call names a generation other than the current job's.
    NotCurrent,
    /// The start names a generation that does not exceed every accepted one.
    NotNewer,
}

/// A codec, renderer or encoder failure reduced to a stable code and message at the
/// boundary. The reduction keeps stored job failures cloneable, because the upstream
/// I/O and encoder errors are not `Clone`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineFailure {
    code: ErrorCode,
    message: String,
}

impl EngineFailure {
    fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Returns the stable code of the reduced failure.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        self.code
    }

    /// Returns the human-readable message. It may contain engine detail but never a
    /// filesystem path or source bytes.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Every failure the session reports. Each variant has one stable [`ErrorCode`] and a
/// fixed message, so the WASM and native shells report identical codes and text.
///
/// Invariant: a value is `Clone`, so a terminal job failure can be stored and returned
/// again by later calls.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionError {
    /// The session was released; every later call fails with this error.
    Disposed,
    /// No job exists, or the job is not for the requested generation.
    NoJob,
    /// No job of the requested kind exists for the generation.
    NoActiveJob(JobKind),
    /// The generation is not the current job, or does not exceed earlier ones.
    StaleGeneration(StaleReason),
    /// A result was requested before the job reached its finished stage.
    JobIncomplete(JobKind),
    /// A job slot held a placeholder while it was being advanced.
    Interrupted,
    /// The job changed stage between the checks and the take during finish.
    Changed,
    /// The preview frame's pixels were already taken.
    FrameConsumed,
    /// An argument failed the shared argument contract; the code and text are fixed by that contract.
    Argument {
        /// Code the argument contract assigns to this failure.
        code: ErrorCode,
        /// Text the browser reports for the same input.
        message: String,
    },
    /// A stage advance was given a zero pixel budget.
    ZeroBudget,
    /// The frame index is outside the source's frames; a still source has only frame `0`.
    FrameOutOfRange {
        /// The requested frame index.
        frame: usize,
        /// How many frames the source holds.
        count: usize,
    },
    /// The frame table could not be reserved.
    FrameTableReservation,
    /// A preview setting is outside its inclusive range.
    SettingOutOfRange {
        /// Browser name of the setting, `cellSize` or `fontSize`.
        setting: &'static str,
        /// Inclusive lower bound.
        min: u8,
        /// Inclusive upper bound.
        max: u8,
    },
    /// Source width or height does not fit the pixel ceiling's representation.
    DimensionsTooLarge,
    /// Source pixel length differs from `width * height * 4`.
    SourceLength,
    /// The source pixel buffer could not be reserved.
    SourceReservation,
    /// The encoded input exceeds the 256 MiB limit.
    EncodedTooLarge,
    /// The encoded input buffer could not be reserved.
    EncodedReservation,
    /// A shared contract check failed; its code follows [`ContractError`].
    Contract(ContractError),
    /// A codec, renderer or encoder failure, already reduced to a code and message.
    Engine(EngineFailure),
}

impl SessionError {
    /// Returns the stable code for this failure.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Disposed => ErrorCode::Disposed,
            Self::NoJob | Self::NoActiveJob(_) => ErrorCode::NoJob,
            Self::StaleGeneration(_) => ErrorCode::StaleGeneration,
            Self::JobIncomplete(_) => ErrorCode::JobIncomplete,
            Self::Interrupted | Self::Changed => ErrorCode::Internal,
            Self::FrameConsumed => ErrorCode::FrameConsumed,
            Self::Argument { code, .. } => *code,
            Self::ZeroBudget | Self::FrameOutOfRange { .. } => ErrorCode::InvalidArgument,
            Self::SettingOutOfRange { .. } => ErrorCode::InvalidSettings,
            Self::DimensionsTooLarge | Self::SourceReservation => ErrorCode::ResourceLimitExceeded,
            Self::SourceLength => ErrorCode::InvalidDimensions,
            Self::EncodedTooLarge => ErrorCode::EncodedTooLarge,
            Self::EncodedReservation | Self::FrameTableReservation => ErrorCode::ReservationFailed,
            Self::Contract(error) => contract_code(error),
            Self::Engine(failure) => failure.code,
        }
    }

    /// Returns the fixed message for this failure. Contract and engine failures
    /// delegate to their own text.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Disposed => "the session has been disposed".to_owned(),
            Self::NoJob => "there is no active job for this generation".to_owned(),
            Self::NoActiveJob(JobKind::Preview) => {
                "there is no active preview for this generation".to_owned()
            }
            Self::NoActiveJob(JobKind::Export) => {
                "there is no active export for this generation".to_owned()
            }
            Self::StaleGeneration(StaleReason::NotCurrent) => {
                "generation does not match the current job".to_owned()
            }
            Self::StaleGeneration(StaleReason::NotNewer) => {
                "generation is not newer than the current job".to_owned()
            }
            Self::JobIncomplete(JobKind::Preview) => {
                "the job has not finished rendering".to_owned()
            }
            Self::JobIncomplete(JobKind::Export) => {
                "the export has not finished encoding".to_owned()
            }
            Self::Interrupted => "the job was interrupted".to_owned(),
            Self::Changed => "job state changed during finish".to_owned(),
            Self::FrameConsumed => "the frame pixels were already taken".to_owned(),
            Self::Argument { message, .. } => message.clone(),
            Self::ZeroBudget => "pixelBudget must be positive".to_owned(),
            Self::FrameOutOfRange { frame, count } => {
                format!("frame {frame} is outside the {count} frame(s) of this source")
            }
            Self::FrameTableReservation => {
                "could not reserve memory for the animation frames".to_owned()
            }
            Self::SettingOutOfRange { setting, min, max } => {
                format!("{setting} must be between {min} and {max}")
            }
            Self::DimensionsTooLarge => "image dimension exceeds the pixel ceiling".to_owned(),
            Self::SourceLength => "pixels must contain exactly width * height * 4 bytes".to_owned(),
            Self::SourceReservation => "could not reserve memory for the source pixels".to_owned(),
            Self::EncodedTooLarge => encoded_too_large_message(),
            Self::EncodedReservation => "could not reserve memory for the encoded input".to_owned(),
            Self::Contract(error) => error.to_string(),
            Self::Engine(failure) => failure.message.clone(),
        }
    }
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code().as_str(), self.message())
    }
}

impl std::error::Error for SessionError {}

fn encoded_too_large_message() -> String {
    format!(
        "encoded input exceeds the {} MiB limit",
        MAX_ENCODED_BYTES / (1024 * 1024)
    )
}

fn contract_code(error: &ContractError) -> ErrorCode {
    match error {
        ContractError::InvalidDimensions => ErrorCode::InvalidDimensions,
        ContractError::ResourceLimitExceeded => ErrorCode::ResourceLimitExceeded,
        ContractError::ArithmeticOverflow => ErrorCode::ArithmeticOverflow,
        ContractError::InvalidSettings(_) | ContractError::InvalidRamp => {
            ErrorCode::InvalidSettings
        }
        ContractError::InvalidProgress => ErrorCode::InvalidProgress,
        ContractError::Cancelled => ErrorCode::Cancelled,
        ContractError::StaleGeneration => ErrorCode::StaleGeneration,
    }
}

impl From<ContractError> for SessionError {
    fn from(error: ContractError) -> Self {
        Self::Contract(error)
    }
}

impl From<RenderError> for SessionError {
    fn from(error: RenderError) -> Self {
        match error {
            RenderError::Contract(inner) => Self::Contract(inner),
            RenderError::InvalidFont(message) => Self::Engine(EngineFailure::new(
                ErrorCode::Internal,
                format!("bundled font is invalid: {message}"),
            )),
        }
    }
}

impl From<DecodeError> for SessionError {
    fn from(error: DecodeError) -> Self {
        match error {
            DecodeError::EncodedInputTooLarge => Self::EncodedTooLarge,
            DecodeError::UnsupportedFormat => Self::Engine(EngineFailure::new(
                ErrorCode::UnsupportedFormat,
                "the image format is not supported",
            )),
            DecodeError::CodecUnavailable(format) => Self::Engine(EngineFailure::new(
                ErrorCode::CodecUnavailable,
                format!("{format} decoding is not available in this build"),
            )),
            DecodeError::Contract(inner) => Self::Contract(inner),
            DecodeError::ResourceLimit(diagnostic) => Self::Engine(EngineFailure::new(
                ErrorCode::ResourceLimitExceeded,
                format!("the image exceeds a decoder resource limit: {diagnostic}"),
            )),
            DecodeError::Image(inner) => Self::Engine(EngineFailure::new(
                ErrorCode::DecodeFailed,
                format!("the image data could not be decoded: {inner}"),
            )),
            DecodeError::Io(inner) => Self::Engine(EngineFailure::new(
                ErrorCode::DecodeFailed,
                format!("the image data could not be read: {inner}"),
            )),
        }
    }
}

impl From<PngTaskError> for SessionError {
    fn from(error: PngTaskError) -> Self {
        match error {
            PngTaskError::Contract(inner) => Self::Contract(inner),
            PngTaskError::Render(inner) => Self::from(inner),
            PngTaskError::Export(inner) => Self::from(inner),
            PngTaskError::Terminated => Self::Engine(EngineFailure::new(
                ErrorCode::Internal,
                "the PNG export was already terminated",
            )),
        }
    }
}

impl From<ExportError> for SessionError {
    fn from(error: ExportError) -> Self {
        match error {
            ExportError::Contract(inner) => Self::Contract(inner),
            ExportError::Row(inner) => Self::from(*inner),
            ExportError::Png(inner) => Self::Engine(EngineFailure::new(
                ErrorCode::EncodeFailed,
                format!("the PNG encoder failed: {inner}"),
            )),
            ExportError::Gif(inner) => Self::Engine(EngineFailure::new(
                ErrorCode::EncodeFailed,
                format!("the GIF encoder failed: {inner}"),
            )),
            ExportError::Sink(reason) => Self::Engine(EngineFailure::new(
                ErrorCode::Internal,
                format!("PNG chunk storage failed: {reason}"),
            )),
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
            assert_eq!(SessionError::from(error).code().as_str(), expected);
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
            assert_eq!(SessionError::from(error).code().as_str(), expected);
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
            assert_eq!(SessionError::from(error).code().as_str(), expected);
        }
    }
}
