//! Typed IPC errors. Codes come from the shared session vocabulary, so the page sees the same
//! `{ code, message }` pairs as the browser.

use std::fmt;

pub use image_to_binary_adapters::session::ErrorCode;
use image_to_binary_adapters::session::SessionError;
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

use crate::worker::WorkerError;

/// Error returned by every fallible IPC command, serialised as `{ code, message }`.
///
/// Invariant: `code` is one of [`ErrorCode`], so the frontend switches on a closed set.
#[derive(Clone, Debug, Eq, PartialEq)]
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

impl Serialize for IpcError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("IpcError", 2)?;
        state.serialize_field("code", self.code.as_str())?;
        state.serialize_field("message", &self.message)?;
        state.end()
    }
}

impl fmt::Display for IpcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for IpcError {}

impl From<SessionError> for IpcError {
    fn from(error: SessionError) -> Self {
        Self::new(error.code(), error.message())
    }
}

impl From<WorkerError> for IpcError {
    fn from(error: WorkerError) -> Self {
        match error {
            WorkerError::Stopped => {
                Self::new(ErrorCode::Cancelled, "the processing worker has stopped")
            }
            WorkerError::Spawn => Self::new(
                ErrorCode::Internal,
                "the processing worker thread could not be started",
            ),
            WorkerError::GenerationExhausted => Self::new(
                ErrorCode::Internal,
                "the processing generation counter is exhausted",
            ),
            WorkerError::JobPanicked => {
                Self::new(ErrorCode::Internal, "the processing job panicked")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image_to_binary_core::ContractError;

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
            let json = serde_json::to_value(IpcError::new(code, "m")).unwrap();
            assert_eq!(json["code"], serde_json::Value::from(code.as_str()));
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
    fn session_failures_keep_their_shared_code_and_message() {
        let error = IpcError::from(SessionError::from(ContractError::Cancelled));
        assert_eq!(error.code(), ErrorCode::Cancelled);
        assert_eq!(error.message(), ContractError::Cancelled.to_string());
    }
}
