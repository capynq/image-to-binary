//! Transport-neutral argument contract shared by the WASM and native shells.
//!
//! Browser numbers are IEEE doubles, so every argument arrives as `f64` (a missing or non-number
//! value is `NaN`). Each conversion checks in the order the browser contract defines and reports
//! the first failure, so both shells return the same code and text for the same input.

use image_to_binary_core::{
    Generation, Ramp, RenderMode, Sampling, MAX_CELL_SIZE, MAX_FONT_SIZE, MAX_RAMP_LENGTH,
    MAX_THRESHOLD, MIN_CELL_SIZE, MIN_FONT_SIZE, MIN_THRESHOLD,
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

/// A text argument as the shells receive it. `Absent` covers a missing, `undefined` or `null`
/// value; `NotText` is any value that is not a string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextArg<'a> {
    /// The argument was omitted or sent as `undefined` or `null`.
    Absent,
    /// The argument is a string.
    Text(&'a str),
    /// The argument has a non-string type.
    NotText,
}

/// The default ramp used by `ascii` mode when no ramp is supplied.
pub const DEFAULT_RAMP: &str = " .:-=+*#%@";

/// Failure text for an empty ramp. The browser pins the same text in `protocol.ts`.
pub const RAMP_EMPTY_MESSAGE: &str = "ramp must contain at least one character";
/// Failure text for a ramp with a byte outside printable ASCII. The browser pins the same text.
pub const RAMP_NOT_PRINTABLE_MESSAGE: &str = "ramp must contain only printable ASCII characters";
/// Failure text for a ramp longer than `MAX_RAMP_LENGTH`. The browser pins the same text.
pub const RAMP_TOO_LONG_MESSAGE: &str = "ramp must be at most 64 characters";

/// Validates preview settings in the browser's order: sampling first, then cell size, font size
/// and threshold, each converted and range-checked before the next is read. Mode and ramp follow:
/// the mode name, then the ramp, then the mode's sampling requirement.
///
/// `sampling` is `None` when the argument is not a string. The sampling names are `classic` and
/// `average`; the mode names are `binary` (the default), `ascii` and `block`. An absent ramp
/// selects [`DEFAULT_RAMP`]. A supplied ramp is validated in every mode, so the same text is
/// accepted or refused whatever the mode, matching the browser's settings validation.
///
/// # Errors
///
/// Returns `invalid_argument` for a missing sampling string or a non-string mode or ramp,
/// `invalid_settings` for an unknown sampling or mode name, an invalid ramp, an `ascii` or `block`
/// mode with a non-`average` sampling, and the conversion errors of [`ranged`] for each size.
pub fn preview_settings(
    cell_size: f64,
    font_size: f64,
    threshold: f64,
    sampling: Option<&str>,
    mode: TextArg<'_>,
    ramp: TextArg<'_>,
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
    let mode = render_mode(mode, ramp)?;
    if mode != RenderMode::Binary && sampling != Sampling::Average {
        return Err(SessionError::Argument {
            code: ErrorCode::InvalidSettings,
            message: "ascii and block modes require average sampling".to_owned(),
        });
    }
    PreviewSettings::new(cell_size, font_size, threshold, sampling)?.with_mode(mode)
}

/// Converts the mode name and, for `ascii`, the ramp, in that order.
fn render_mode(mode: TextArg<'_>, ramp: TextArg<'_>) -> Result<RenderMode, SessionError> {
    let name = match mode {
        TextArg::Absent => "binary",
        TextArg::Text(name) => name,
        TextArg::NotText => {
            return Err(SessionError::Argument {
                code: ErrorCode::InvalidArgument,
                message: "mode must be a string".to_owned(),
            })
        }
    };
    let ramp = match ramp {
        TextArg::Absent => parse_ramp(DEFAULT_RAMP)?,
        TextArg::Text(text) => parse_ramp(text)?,
        TextArg::NotText => {
            return Err(SessionError::Argument {
                code: ErrorCode::InvalidArgument,
                message: "ramp must be a string".to_owned(),
            })
        }
    };
    match name {
        "binary" => Ok(RenderMode::Binary),
        "block" => Ok(RenderMode::Block),
        "ascii" => Ok(RenderMode::Ascii(ramp)),
        _ => Err(SessionError::Argument {
            code: ErrorCode::InvalidSettings,
            message: "mode must be \"binary\", \"ascii\" or \"block\"".to_owned(),
        }),
    }
}

/// Validates a custom ramp with a readable message for each failure, in this order: empty,
/// non-printable or non-ASCII, then longer than the shared maximum.
///
/// # Errors
///
/// Returns `invalid_settings` with the text the browser shows for the same ramp.
pub fn parse_ramp(text: &str) -> Result<Ramp, SessionError> {
    let message = if text.is_empty() {
        RAMP_EMPTY_MESSAGE
    } else if !text.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
        RAMP_NOT_PRINTABLE_MESSAGE
    } else if text.len() > MAX_RAMP_LENGTH {
        RAMP_TOO_LONG_MESSAGE
    } else {
        return Ramp::new(text).map_err(SessionError::from);
    };
    Err(SessionError::Argument {
        code: ErrorCode::InvalidSettings,
        message: message.to_owned(),
    })
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
    use image_to_binary_core::MAX_RAMP_LENGTH;

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
                code(preview_settings(
                    bad,
                    10.0,
                    128.0,
                    Some("classic"),
                    TextArg::Absent,
                    TextArg::Absent
                )),
                ErrorCode::InvalidArgument
            );
            assert_eq!(
                code(preview_settings(
                    5.0,
                    10.0,
                    bad,
                    Some("classic"),
                    TextArg::Absent,
                    TextArg::Absent
                )),
                ErrorCode::InvalidArgument
            );
        }
        assert_eq!(
            code(preview_settings(
                300.0,
                10.0,
                128.0,
                Some("classic"),
                TextArg::Absent,
                TextArg::Absent
            )),
            ErrorCode::InvalidSettings
        );
        assert_eq!(
            code(preview_settings(
                5.0,
                10.0,
                128.0,
                Some("median"),
                TextArg::Absent,
                TextArg::Absent
            )),
            ErrorCode::InvalidSettings
        );
        assert_eq!(
            code(preview_settings(
                5.0,
                10.0,
                128.0,
                None,
                TextArg::Absent,
                TextArg::Absent
            )),
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn range_errors_name_the_setting_as_the_browser_does() {
        assert_eq!(
            text(preview_settings(
                4.0,
                10.0,
                128.0,
                Some("classic"),
                TextArg::Absent,
                TextArg::Absent
            )),
            "cellSize must be between 5 and 30"
        );
        assert_eq!(
            text(preview_settings(
                5.0,
                31.0,
                128.0,
                Some("classic"),
                TextArg::Absent,
                TextArg::Absent
            )),
            "fontSize must be between 8 and 30"
        );
        assert_eq!(
            text(preview_settings(
                5.0,
                10.0,
                300.0,
                Some("classic"),
                TextArg::Absent,
                TextArg::Absent
            )),
            "threshold must be between 0 and 255"
        );
        assert_eq!(
            text(preview_settings(
                5.0,
                10.0,
                128.0,
                Some("median"),
                TextArg::Absent,
                TextArg::Absent
            )),
            "sampling must be \"classic\" or \"average\""
        );
        assert_eq!(
            text(preview_settings(
                5.0,
                10.0,
                128.0,
                None,
                TextArg::Absent,
                TextArg::Absent
            )),
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

    fn mode_of(result: Result<PreviewSettings, SessionError>) -> RenderMode {
        result.unwrap().mode
    }

    #[test]
    fn the_message_constants_are_derived_from_the_shared_maximum() {
        assert_eq!(
            RAMP_TOO_LONG_MESSAGE,
            format!("ramp must be at most {MAX_RAMP_LENGTH} characters")
        );
    }

    #[test]
    fn omitted_mode_and_ramp_are_classic_binary() {
        let settings = preview_settings(
            5.0,
            10.0,
            128.0,
            Some("classic"),
            TextArg::Absent,
            TextArg::Absent,
        )
        .unwrap();
        assert_eq!(
            settings,
            PreviewSettings::new(5, 10, 128, Sampling::Classic).unwrap()
        );
        assert_eq!(settings.mode, RenderMode::Binary);
        assert_eq!(
            mode_of(preview_settings(
                5.0,
                10.0,
                128.0,
                Some("average"),
                TextArg::Text("binary"),
                TextArg::Absent,
            )),
            RenderMode::Binary
        );
    }

    #[test]
    fn ascii_without_a_ramp_uses_the_shared_default() {
        assert_eq!(DEFAULT_RAMP, Ramp::DEFAULT_ASCII.as_str());
        assert_eq!(
            mode_of(preview_settings(
                5.0,
                10.0,
                128.0,
                Some("average"),
                TextArg::Text("ascii"),
                TextArg::Absent,
            )),
            RenderMode::Ascii(Ramp::DEFAULT_ASCII)
        );
    }

    #[test]
    fn custom_ascii_and_block_modes_are_accepted_with_average_sampling() {
        assert_eq!(
            mode_of(preview_settings(
                5.0,
                10.0,
                128.0,
                Some("average"),
                TextArg::Text("ascii"),
                TextArg::Text(" .#@"),
            )),
            RenderMode::Ascii(Ramp::new(" .#@").unwrap())
        );
        assert_eq!(
            mode_of(preview_settings(
                5.0,
                10.0,
                128.0,
                Some("average"),
                TextArg::Text("block"),
                TextArg::Absent,
            )),
            RenderMode::Block
        );
    }

    #[test]
    fn invalid_ramps_name_the_failure_as_invalid_settings() {
        let oversized = "a".repeat(MAX_RAMP_LENGTH + 1);
        let cases = [
            ("", "ramp must contain at least one character"),
            (
                "\u{e9}",
                "ramp must contain only printable ASCII characters",
            ),
            ("a\tb", "ramp must contain only printable ASCII characters"),
            (
                "a\u{7f}",
                "ramp must contain only printable ASCII characters",
            ),
            (oversized.as_str(), "ramp must be at most 64 characters"),
        ];
        for (ramp, expected) in cases {
            let result = preview_settings(
                5.0,
                10.0,
                128.0,
                Some("average"),
                TextArg::Text("ascii"),
                TextArg::Text(ramp),
            );
            assert_eq!(code(result.clone()), ErrorCode::InvalidSettings, "{ramp:?}");
            assert_eq!(text(result), expected, "{ramp:?}");
        }
    }

    #[test]
    fn ramp_of_sixty_four_characters_is_the_last_accepted_length() {
        let full: String = (0x21_u8..0x21 + 64).map(char::from).collect();
        assert!(preview_settings(
            5.0,
            10.0,
            128.0,
            Some("average"),
            TextArg::Text("ascii"),
            TextArg::Text(&full),
        )
        .is_ok());
    }

    #[test]
    fn ascii_and_block_require_average_sampling() {
        for mode in ["ascii", "block"] {
            let result = preview_settings(
                5.0,
                10.0,
                128.0,
                Some("classic"),
                TextArg::Text(mode),
                TextArg::Absent,
            );
            assert_eq!(code(result.clone()), ErrorCode::InvalidSettings, "{mode}");
            assert_eq!(
                text(result),
                "ascii and block modes require average sampling",
                "{mode}"
            );
        }
    }

    #[test]
    fn unknown_and_non_string_modes_and_ramps_are_typed() {
        assert_eq!(
            code(preview_settings(
                5.0,
                10.0,
                128.0,
                Some("average"),
                TextArg::Text("dots"),
                TextArg::Absent,
            )),
            ErrorCode::InvalidSettings
        );
        assert_eq!(
            text(preview_settings(
                5.0,
                10.0,
                128.0,
                Some("average"),
                TextArg::Text("dots"),
                TextArg::Absent,
            )),
            "mode must be \"binary\", \"ascii\" or \"block\""
        );
        assert_eq!(
            code(preview_settings(
                5.0,
                10.0,
                128.0,
                Some("average"),
                TextArg::NotText,
                TextArg::Absent,
            )),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            code(preview_settings(
                5.0,
                10.0,
                128.0,
                Some("average"),
                TextArg::Text("ascii"),
                TextArg::NotText,
            )),
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn the_ramp_is_validated_in_every_mode() {
        for mode in ["binary", "block", "ascii"] {
            assert_eq!(
                code(preview_settings(
                    5.0,
                    10.0,
                    128.0,
                    Some("average"),
                    TextArg::Text(mode),
                    TextArg::Text(""),
                )),
                ErrorCode::InvalidSettings,
                "{mode}"
            );
            assert_eq!(
                text(preview_settings(
                    5.0,
                    10.0,
                    128.0,
                    Some("average"),
                    TextArg::Text(mode),
                    TextArg::Text("caf\u{e9}"),
                )),
                RAMP_NOT_PRINTABLE_MESSAGE,
                "{mode}"
            );
        }
        assert!(preview_settings(
            5.0,
            10.0,
            128.0,
            Some("classic"),
            TextArg::Text("binary"),
            TextArg::Text("#"),
        )
        .is_ok());
    }
}
