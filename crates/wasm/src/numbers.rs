//! Strict conversion of JS numbers into bounded integers.
//!
//! Non-number JS values are mapped to NaN before reaching this layer, so
//! nothing here can wrap, truncate, or accept fractional or negative input.

use crate::error::{BindingError, ErrorCode};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_SAFE_FLOAT: f64 = 9_007_199_254_740_991.0;

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(crate) fn safe_integer(value: f64, name: &str) -> Result<u64, BindingError> {
    if value.is_finite() && value.fract() == 0.0 && (0.0..=MAX_SAFE_FLOAT).contains(&value) {
        Ok(value as u64)
    } else {
        Err(BindingError::new(
            ErrorCode::InvalidArgument,
            format!("{name} must be a non-negative safe integer"),
        ))
    }
}

pub(crate) fn positive(value: f64, name: &str) -> Result<u64, BindingError> {
    match safe_integer(value, name)? {
        0 => Err(BindingError::new(
            ErrorCode::InvalidArgument,
            format!("{name} must be positive"),
        )),
        integer => Ok(integer),
    }
}

pub(crate) fn ranged(value: f64, name: &str, min: u8, max: u8) -> Result<u8, BindingError> {
    let integer = safe_integer(value, name)?;
    match u8::try_from(integer) {
        Ok(narrow) if (min..=max).contains(&narrow) => Ok(narrow),
        _ => Err(BindingError::new(
            ErrorCode::InvalidSettings,
            format!("{name} must be between {min} and {max}"),
        )),
    }
}

#[allow(clippy::cast_precision_loss)]
pub(crate) fn js_count(value: u64) -> f64 {
    value.min(MAX_SAFE_INTEGER) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_integer_rejects_everything_that_could_wrap_or_truncate() {
        for bad in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            -1.0,
            -251.0,
            1.5,
            9_007_199_254_740_992.0,
        ] {
            assert_eq!(
                safe_integer(bad, "n").unwrap_err().code(),
                ErrorCode::InvalidArgument
            );
        }
        assert_eq!(safe_integer(0.0, "n").unwrap(), 0);
        assert_eq!(safe_integer(-0.0, "n").unwrap(), 0);
        assert_eq!(safe_integer(MAX_SAFE_FLOAT, "n").unwrap(), MAX_SAFE_INTEGER);
        assert!(positive(0.0, "n").is_err());
        assert_eq!(
            ranged(300.0, "n", 5, 30).unwrap_err().code(),
            ErrorCode::InvalidSettings
        );
        assert_eq!(
            ranged(-251.0, "n", 0, 255).unwrap_err().code(),
            ErrorCode::InvalidArgument
        );
        assert_eq!(ranged(255.0, "n", 0, 255).unwrap(), 255);
    }
}
