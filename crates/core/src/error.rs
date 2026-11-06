use core::fmt;

use crate::MAX_RAMP_LENGTH;

/// Failure returned when a contract value cannot be represented or validated.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContractError {
    /// A dimension or derived size was zero or exceeded a configured limit.
    InvalidDimensions,
    /// A valid value exceeded an explicit resource ceiling.
    ResourceLimitExceeded,
    /// A checked arithmetic operation overflowed.
    ArithmeticOverflow,
    /// A value did not satisfy the settings contract.
    InvalidSettings(&'static str),
    /// A custom ramp contains a non-printable character or is too long.
    InvalidRamp,
    /// Progress was advanced beyond its declared total.
    InvalidProgress,
    /// Work was cancelled for its generation.
    Cancelled,
    /// A result belongs to an older generation and must be discarded.
    StaleGeneration,
}

impl fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions => formatter.write_str("dimensions are invalid"),
            Self::ResourceLimitExceeded => formatter.write_str("resource limit exceeded"),
            Self::ArithmeticOverflow => formatter.write_str("arithmetic overflow"),
            Self::InvalidSettings(field) => write!(formatter, "invalid settings: {field}"),
            Self::InvalidRamp => write!(
                formatter,
                "ramp must be printable ASCII and at most {MAX_RAMP_LENGTH} characters"
            ),
            Self::InvalidProgress => formatter.write_str("progress exceeds its total"),
            Self::Cancelled => formatter.write_str("operation cancelled"),
            Self::StaleGeneration => formatter.write_str("operation belongs to a stale generation"),
        }
    }
}

impl std::error::Error for ContractError {}
