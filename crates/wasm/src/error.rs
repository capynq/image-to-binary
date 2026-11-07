//! Maps the shared session failures onto the `{ code, message }` pair the JS boundary throws.

use std::fmt;

pub(crate) use image_to_binary_adapters::session::ErrorCode;
use image_to_binary_adapters::session::SessionError;

/// A failure ready for the JS boundary: a stable code and the message the Error carries.
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

impl From<SessionError> for BindingError {
    fn from(error: SessionError) -> Self {
        Self::new(error.code(), error.message())
    }
}
