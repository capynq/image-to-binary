//! Colour and tone contracts applied at render time, never during sampling.

use image_to_binary_core::{ContractError, MAX_CONTRAST, MIN_CONTRAST};

use crate::{RenderOptions, SampleCell};

const MIDPOINT: i32 = 128;
const PERCENT: i32 = 100;
const ALPHA_SCALE: u32 = 255;

/// Straight 8-bit RGB colour without alpha.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rgb {
    /// Red channel.
    pub red: u8,
    /// Green channel.
    pub green: u8,
    /// Blue channel.
    pub blue: u8,
}

/// Selects where each glyph's foreground colour comes from.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ColorMode {
    /// Every glyph uses [`RenderStyle::foreground`].
    #[default]
    Monochrome,
    /// Each cell uses its sampled RGB, matted over the background by its sampled alpha.
    ///
    /// The matte is applied at render time as `(c * a + bg * (255 - a) + 127) / 255`
    /// per channel, so the output stays fully opaque and sampled grids are reusable.
    /// A fully transparent cell renders as the exact background; an opaque cell renders
    /// as its exact RGB.
    Source,
}

/// Colour, contrast, and invert controls applied after sampling.
///
/// Contrast must lie in `MIN_CONTRAST..=MAX_CONTRAST`; render entry points reject
/// other values with [`ContractError::InvalidSettings`]. The output alpha is always 255.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderStyle {
    /// Glyph colour under [`ColorMode::Monochrome`].
    pub foreground: Rgb,
    /// Background under every colour mode.
    pub background: Rgb,
    /// Source of each cell's foreground colour.
    pub color: ColorMode,
    /// Contrast in percent around mid-grey; `0` is the identity.
    pub contrast: i8,
    /// Inverts luminance after contrast is applied.
    pub invert: bool,
}

impl Default for RenderStyle {
    fn default() -> Self {
        Self {
            foreground: Rgb {
                red: 0,
                green: 255,
                blue: 0,
            },
            background: Rgb {
                red: 0,
                green: 0,
                blue: 0,
            },
            color: ColorMode::Monochrome,
            contrast: 0,
            invert: false,
        }
    }
}

/// Returns the single luminance that thresholds and ramps read for one cell.
///
/// Total for every `RenderStyle`: contrast outside `MIN_CONTRAST..=MAX_CONTRAST`
/// is clamped to those shared bounds before use, so unvalidated direct callers
/// get the same tone as the nearest accepted value.
///
/// Contrast is applied first, then invert, so the order is observable. Contrast
/// is `128 + round(delta * (100 + c) / 100)` where `delta = luma - 128`, clamped
/// to `0..=255`. The rounding rule is half away from zero, computed on integers.
/// Contrast `0` with invert disabled returns `luma` exactly, and contrast `-100`
/// returns `128` for every input.
#[must_use]
pub fn toned_luma(cell: SampleCell, options: &RenderOptions) -> u8 {
    let contrasted = contrast_luma(cell.luma(), options.style.contrast);
    if options.style.invert {
        u8::MAX - contrasted
    } else {
        contrasted
    }
}

fn contrast_luma(luma: u8, contrast: i8) -> u8 {
    let contrast = i32::from(contrast).clamp(i32::from(MIN_CONTRAST), i32::from(MAX_CONTRAST));
    let delta = i32::from(luma) - MIDPOINT;
    let scaled = delta * (PERCENT + contrast);
    let magnitude = (scaled.abs() + PERCENT / 2) / PERCENT;
    let signed = if scaled < 0 { -magnitude } else { magnitude };
    let value = (MIDPOINT + signed).clamp(0, i32::from(u8::MAX));
    u8::try_from(value).unwrap_or(u8::MAX)
}

/// Mixes `foreground` over `background` as `(fg * weight + bg * (scale - weight) + scale / 2) / scale`.
///
/// Callers guarantee `weight <= scale`; a larger weight is an arithmetic error.
pub(crate) fn mix(
    foreground: Rgb,
    background: Rgb,
    weight: u32,
    scale: u32,
) -> Result<[u8; 3], ContractError> {
    let inverse = scale
        .checked_sub(weight)
        .ok_or(ContractError::ArithmeticOverflow)?;
    let channels = [
        (foreground.red, background.red),
        (foreground.green, background.green),
        (foreground.blue, background.blue),
    ];
    let mut output = [0; 3];
    for (value, (fore, back)) in output.iter_mut().zip(channels) {
        let total = u32::from(fore)
            .checked_mul(weight)
            .zip(u32::from(back).checked_mul(inverse))
            .and_then(|(front, rear)| front.checked_add(rear))
            .and_then(|sum| sum.checked_add(scale / 2))
            .ok_or(ContractError::ArithmeticOverflow)?;
        *value = u8::try_from(total / scale).map_err(|_| ContractError::ArithmeticOverflow)?;
    }
    Ok(output)
}

/// Mattes a straight source colour over the background by its alpha.
pub(crate) fn matte(color: [u8; 4], background: Rgb) -> Result<Rgb, ContractError> {
    let foreground = Rgb {
        red: color[0],
        green: color[1],
        blue: color[2],
    };
    let weight = u32::from(color[3]);
    let [red, green, blue] = mix(foreground, background, weight, ALPHA_SCALE)?;
    Ok(Rgb { red, green, blue })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(luma: u8) -> SampleCell {
        SampleCell::new(luma, [luma, luma, luma, 255])
    }

    fn options_with(contrast: i8, invert: bool) -> RenderOptions {
        RenderOptions {
            style: RenderStyle {
                contrast,
                invert,
                ..RenderStyle::default()
            },
            ..RenderOptions::default()
        }
    }

    #[test]
    fn zero_contrast_without_invert_is_identity_for_every_luma() {
        let options = options_with(0, false);
        for luma in 0..=255_u8 {
            assert_eq!(toned_luma(cell(luma), &options), luma);
        }
    }

    #[test]
    fn minimum_contrast_flattens_every_luma_to_mid_grey() {
        let options = options_with(-100, false);
        for luma in 0..=255_u8 {
            assert_eq!(toned_luma(cell(luma), &options), 128, "luma {luma}");
        }
    }

    #[test]
    fn maximum_contrast_doubles_distance_and_clamps() {
        let options = options_with(100, false);
        assert_eq!(toned_luma(cell(128), &options), 128);
        assert_eq!(toned_luma(cell(160), &options), 192);
        assert_eq!(toned_luma(cell(100), &options), 72);
        assert_eq!(toned_luma(cell(64), &options), 0);
        assert_eq!(toned_luma(cell(0), &options), 0);
        assert_eq!(toned_luma(cell(192), &options), 255);
        assert_eq!(toned_luma(cell(255), &options), 255);
    }

    #[test]
    fn contrast_rounds_half_away_from_zero() {
        let options = options_with(-50, false);
        assert_eq!(toned_luma(cell(128), &options), 128);
        assert_eq!(toned_luma(cell(129), &options), 129);
        assert_eq!(toned_luma(cell(127), &options), 127);
        assert_eq!(toned_luma(cell(131), &options), 130);
        assert_eq!(toned_luma(cell(125), &options), 126);
        assert_eq!(toned_luma(cell(255), &options), 192);
        assert_eq!(toned_luma(cell(0), &options), 64);
    }

    #[test]
    fn contrast_is_monotone_non_decreasing_for_every_percent_step() {
        for contrast in (-100..=100).step_by(1) {
            let options = options_with(i8::try_from(contrast).unwrap(), false);
            let mut previous = 0;
            for luma in 0..=255_u8 {
                let value = toned_luma(cell(luma), &options);
                assert!(value >= previous, "contrast {contrast} luma {luma}");
                previous = value;
            }
        }
    }

    #[test]
    fn invert_is_exact_complement_of_the_untoned_value() {
        let options = options_with(0, true);
        for luma in 0..=255_u8 {
            assert_eq!(toned_luma(cell(luma), &options), 255 - luma);
        }
    }

    #[test]
    fn out_of_range_contrast_clamps_to_the_shared_bounds() {
        for luma in 0..=255_u8 {
            assert_eq!(
                toned_luma(cell(luma), &options_with(i8::MIN, false)),
                toned_luma(cell(luma), &options_with(MIN_CONTRAST, false)),
                "luma {luma}"
            );
            assert_eq!(
                toned_luma(cell(luma), &options_with(i8::MAX, false)),
                toned_luma(cell(luma), &options_with(MAX_CONTRAST, false)),
                "luma {luma}"
            );
        }
        assert_eq!(toned_luma(cell(0), &options_with(i8::MIN, false)), 128);
        assert_eq!(toned_luma(cell(0), &options_with(i8::MIN, true)), 127);
    }

    #[test]
    fn contrast_applies_before_invert() {
        let contrast_then_invert = toned_luma(cell(100), &options_with(100, true));
        assert_eq!(contrast_then_invert, 183);
        let invert_then_contrast = contrast_luma(255 - 100, 100);
        assert_eq!(invert_then_contrast, 182);
        assert_ne!(contrast_then_invert, invert_then_contrast);
    }

    #[test]
    fn matte_gives_exact_color_when_opaque_and_background_when_transparent() {
        let background = Rgb {
            red: 10,
            green: 20,
            blue: 30,
        };
        let color = [200, 100, 50];
        assert_eq!(
            matte([color[0], color[1], color[2], 255], background).unwrap(),
            Rgb {
                red: 200,
                green: 100,
                blue: 50
            }
        );
        assert_eq!(
            matte([color[0], color[1], color[2], 0], background).unwrap(),
            background
        );
    }

    #[test]
    fn matte_half_alpha_matches_the_documented_formula() {
        let background = Rgb {
            red: 0,
            green: 0,
            blue: 0,
        };
        let blended = matte([255, 255, 255, 128], background).unwrap();
        assert_eq!(blended.red, 128);
        assert_eq!(blended.green, 128);
        assert_eq!(blended.blue, 128);
    }
}
