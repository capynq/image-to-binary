//! Transport-neutral argument contract shared by the WASM and native shells.
//!
//! Browser numbers are IEEE doubles, so every argument arrives as `f64` (a missing or non-number
//! value is `NaN`). Each conversion checks in the order the browser contract defines and reports
//! the first failure, so both shells return the same code and text for the same input.

use image_to_binary_core::{
    Generation, Sampling, MAX_CELL_SIZE, MAX_FONT_SIZE, MAX_THRESHOLD, MIN_CELL_SIZE,
    MIN_FONT_SIZE, MIN_THRESHOLD,
};

use super::error::{ErrorCode, SessionError};
use super::plan::SourcePlan;
use super::settings::PreviewSettings;

/// Largest integer a JavaScript number holds exactly (`Number.MAX_SAFE_INTEGER`).
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_SAFE_FLOAT: f64 = 9_007_199_254_740_991.0;

/// Converts a number into a non-negative safe integer.
///
/// # Errors
///
/// Returns `invalid_argument` with `"{name} must be a non-negative safe integer"` for a
/// non-finite, fractional, negative or too-large value.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn safe_integer(value: f64, name: &str) -> Result<u64, SessionError> {
    if value.is_finite() && value.fract() == 0.0 && (0.0..=MAX_SAFE_FLOAT).contains(&value) {
        Ok(value as u64)
    } else {
        Err(SessionError::Argument {
            code: ErrorCode::InvalidArgument,
            message: format!("{name} must be a non-negative safe integer"),
        })
    }
}

/// Converts a number into a positive safe integer.
///
/// # Errors
///
/// Returns the errors of [`safe_integer`], and `invalid_argument` with `"{name} must be
/// positive"` for zero.
pub fn positive(value: f64, name: &str) -> Result<u64, SessionError> {
    match safe_integer(value, name)? {
        0 => Err(SessionError::Argument {
            code: ErrorCode::InvalidArgument,
            message: format!("{name} must be positive"),
        }),
        integer => Ok(integer),
    }
}

/// Converts a number into a byte-sized setting within `min..=max`.
///
/// # Errors
///
/// Returns the errors of [`safe_integer`] first, then `invalid_settings` with `"{name} must be
/// between {min} and {max}"` for an integer outside the range.
pub fn ranged(value: f64, name: &str, min: u8, max: u8) -> Result<u8, SessionError> {
    let integer = safe_integer(value, name)?;
    match u8::try_from(integer) {
        Ok(narrow) if (min..=max).contains(&narrow) => Ok(narrow),
        _ => Err(SessionError::Argument {
            code: ErrorCode::InvalidSettings,
            message: format!("{name} must be between {min} and {max}"),
        }),
    }
}

/// Converts the page's generation.
///
/// # Errors
///
/// Returns the errors of [`safe_integer`] for the name `generation`.
pub fn generation(value: f64) -> Result<Generation, SessionError> {
    safe_integer(value, "generation").map(Generation::new)
}

/// Converts the per-advance pixel budget.
///
/// # Errors
///
/// Returns the errors of [`positive`] for the name `pixelBudget`.
pub fn pixel_budget(value: f64) -> Result<u64, SessionError> {
    positive(value, "pixelBudget")
}

/// Validates preview settings in the browser's order: sampling first, then cell size, font size
/// and threshold, each converted and range-checked before the next is read.
///
/// `sampling` is `None` when the argument is not a string. The sampling names are `classic` and
/// `average`.
///
/// # Errors
///
/// Returns `invalid_argument` for a missing sampling string, `invalid_settings` for an unknown
/// sampling name, and the conversion errors of [`ranged`] for each size.
pub fn preview_settings(
    cell_size: f64,
    font_size: f64,
    threshold: f64,
    sampling: Option<&str>,
) -> Result<PreviewSettings, SessionError> {
    let sampling = match sampling {
        Some("classic") => Sampling::Classic,
        Some("average") => Sampling::Average,
        Some(_) => {
            return Err(SessionError::Argument {
                code: ErrorCode::InvalidSettings,
                message: "sampling must be \"classic\" or \"average\"".to_owned(),
            })
        }
        None => {
            return Err(SessionError::Argument {
                code: ErrorCode::InvalidArgument,
                message: "sampling must be a string".to_owned(),
            })
        }
    };
    let cell_size = ranged(cell_size, "cellSize", MIN_CELL_SIZE, MAX_CELL_SIZE)?;
    let font_size = ranged(font_size, "fontSize", MIN_FONT_SIZE, MAX_FONT_SIZE)?;
    let threshold = ranged(threshold, "threshold", MIN_THRESHOLD, MAX_THRESHOLD)?;
    PreviewSettings::new(cell_size, font_size, threshold, sampling)
}

/// Validates source geometry: width, height, then the optional pixel ceiling, each converted
/// before the next.
///
/// # Errors
///
/// Returns the conversion errors of [`safe_integer`] and the geometry errors of
/// [`SourcePlan::new`].
pub fn source_plan(
    width: f64,
    height: f64,
    max_pixels: Option<f64>,
) -> Result<SourcePlan, SessionError> {
    let width = safe_integer(width, "width")?;
    let height = safe_integer(height, "height")?;
    let max_pixels = max_pixels
        .map(|ceiling| safe_integer(ceiling, "maxPixels"))
        .transpose()?;
    SourcePlan::new(width, height, max_pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code<T: std::fmt::Debug>(result: Result<T, SessionError>) -> ErrorCode {
        result.unwrap_err().code()
    }

    fn text<T: std::fmt::Debug>(result: Result<T, SessionError>) -> String {
        result.unwrap_err().message()
    }

    #[test]
    fn non_finite_and_fractional_settings_are_invalid_arguments() {
        for bad in [f64::NAN, f64::INFINITY, -251.0, 1.5] {
            assert_eq!(
                code(preview_settings(bad, 10.0, 128.0, Some("classic"))),
                ErrorCode::InvalidArgument
            );
            assert_eq!(
                code(preview_settings(5.0, 10.0, bad, Some("classic"))),
                ErrorCode::InvalidArgument
            );
        }
        assert_eq!(
            code(preview_settings(300.0, 10.0, 128.0, Some("classic"))),
            ErrorCode::InvalidSettings
        );
        assert_eq!(
            code(preview_settings(5.0, 10.0, 128.0, Some("median"))),
            ErrorCode::InvalidSettings
        );
        assert_eq!(
            code(preview_settings(5.0, 10.0, 128.0, None)),
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn range_errors_name_the_setting_as_the_browser_does() {
        assert_eq!(
            text(preview_settings(4.0, 10.0, 128.0, Some("classic"))),
            "cellSize must be between 5 and 30"
        );
        assert_eq!(
            text(preview_settings(5.0, 31.0, 128.0, Some("classic"))),
            "fontSize must be between 8 and 30"
        );
        assert_eq!(
            text(preview_settings(5.0, 10.0, 300.0, Some("classic"))),
            "threshold must be between 0 and 255"
        );
        assert_eq!(
            text(preview_settings(5.0, 10.0, 128.0, Some("median"))),
            "sampling must be \"classic\" or \"average\""
        );
        assert_eq!(
            text(preview_settings(5.0, 10.0, 128.0, None)),
            "sampling must be a string"
        );
    }

    #[test]
    fn non_finite_generations_and_budgets_are_invalid_arguments() {
        for bad in [f64::NAN, f64::INFINITY, -1.0, 1.5] {
            assert_eq!(code(generation(bad)), ErrorCode::InvalidArgument);
            assert_eq!(
                text(generation(bad)),
                "generation must be a non-negative safe integer"
            );
        }
        assert_eq!(generation(0.0).unwrap(), Generation::new(0));
        for bad in [0.0, -1.0, f64::NAN, 1.5] {
            assert_eq!(code(pixel_budget(bad)), ErrorCode::InvalidArgument);
        }
        assert_eq!(text(pixel_budget(0.0)), "pixelBudget must be positive");
    }

    #[test]
    fn non_integer_source_geometry_is_invalid_arguments() {
        assert_eq!(
            code(source_plan(f64::NAN, 2.0, None)),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            code(source_plan(2.0, 2.0, Some(f64::NAN))),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            code(source_plan(2.0, 2.0, Some(0.0))),
            ErrorCode::InvalidSettings
        );
        assert_eq!(
            code(source_plan(2.0, 2.0, Some(50_000_001.0))),
            ErrorCode::InvalidSettings
        );
    }
}
