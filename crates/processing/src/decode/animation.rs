//! Animated GIF decoding into composited full-canvas RGBA8 frames.

use std::io::Cursor;
use std::sync::Arc;

use gif::{DecodeOptions, Repeat};
use image::codecs::gif::GifDecoder;
use image::error::{DecodingError, ImageFormatHint};
use image::{AnimationDecoder, Delay, ImageDecoder, ImageError, ImageFormat, Limits};
use image_to_binary_core::{ContractError, Dimensions, SourceMetadata};

use super::{dimension_ceiling, DecodeError, DecodeLimits, DecodedSource};

/// Largest number of frames accepted for one animation.
///
/// Inputs with more frames fail with [`DecodeError::ResourceLimit`] before
/// any frame is composited.
pub const MAX_ANIMATION_FRAMES: usize = 1000;

const GIF87A: &[u8] = b"GIF87a";
const GIF89A: &[u8] = b"GIF89a";
const DEFAULT_DELAY_CENTISECONDS: u16 = 10;

/// Number of times an animation repeats after its first play.
///
/// Mirrors the GIF NETSCAPE2.0 loop extension. A stored count of `0` means
/// loop forever. `Finite(n)` means play `n` additional times after the first
/// play. `Finite(0)` means play once, and is reported when the extension is
/// absent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoopCount {
    /// Repeat until playback is stopped.
    Infinite,
    /// Repeat this many times after the first play; `0` plays once.
    Finite(u16),
}

/// One composited full-canvas frame of an animation.
///
/// The pixels are shared through [`Arc`] so previews and exports can hold the
/// same frame without copying. Delays of `0` or `1` centisecond are normalized
/// to 10 centiseconds (100 ms).
#[derive(Debug, Eq, PartialEq)]
pub struct AnimationFrame {
    source: Arc<DecodedSource>,
    delay_centiseconds: u16,
}

impl AnimationFrame {
    /// Returns the composited canvas pixels for this frame.
    #[must_use]
    pub const fn source(&self) -> &Arc<DecodedSource> {
        &self.source
    }

    /// Returns the display time in centiseconds, never `0` or `1`.
    #[must_use]
    pub const fn delay_centiseconds(&self) -> u16 {
        self.delay_centiseconds
    }
}

/// A GIF decoded into ordered, full-canvas RGBA8 frames.
///
/// Every frame has the canvas dimensions described by `metadata`. Frames are
/// in presentation order and the sequence always has at least two entries and
/// at most [`MAX_ANIMATION_FRAMES`]. Values are produced only by
/// [`decode_animation`], which enforces the frame and decoded-byte budgets.
#[derive(Debug, Eq, PartialEq)]
pub struct DecodedAnimation {
    metadata: SourceMetadata,
    frames: Vec<AnimationFrame>,
    loop_count: LoopCount,
}

impl DecodedAnimation {
    /// Returns the canvas metadata shared by every frame.
    #[must_use]
    pub const fn metadata(&self) -> SourceMetadata {
        self.metadata
    }

    /// Returns the frames in presentation order.
    #[must_use]
    pub fn frames(&self) -> &[AnimationFrame] {
        &self.frames
    }

    /// Returns how many times the animation repeats.
    #[must_use]
    pub const fn loop_count(&self) -> LoopCount {
        self.loop_count
    }
}

/// Decodes an animated GIF into composited full-canvas frames.
///
/// Returns `Ok(None)` when `bytes` is not a GIF or the GIF has at most one
/// frame, so callers can fall back to [`decode`](super::decode) for stills.
/// The frame count and decoded footprint are checked before each frame is
/// accepted. The footprint is every frame plus one extra canvas for the
/// decoder's working buffer, and must fit `limits.max_decoded_bytes`.
///
/// # Errors
///
/// Returns [`DecodeError::EncodedInputTooLarge`] above the encoded-byte limit,
/// [`DecodeError::ResourceLimit`] when the animation has more than
/// [`MAX_ANIMATION_FRAMES`] frames, [`DecodeError::Contract`] when the pixel
/// ceiling or decoded-byte budget is exceeded, and [`DecodeError::Image`] for
/// malformed GIF data.
pub fn decode_animation(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<Option<DecodedAnimation>, DecodeError> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limits.max_encoded_bytes {
        return Err(DecodeError::EncodedInputTooLarge);
    }
    if !(bytes.starts_with(GIF87A) || bytes.starts_with(GIF89A)) {
        return Ok(None);
    }
    let header = scan_gif(bytes)?;
    if header.frame_count <= 1 {
        return Ok(None);
    }
    let dimensions = Dimensions::new(header.width, header.height)?;
    let frame_bytes = u64::try_from(limits.rgba_len(dimensions)?)
        .map_err(|_| ContractError::ArithmeticOverflow)?;

    let ceiling = dimension_ceiling(limits)?;
    let mut image_limits = Limits::default();
    image_limits.max_image_width = Some(ceiling);
    image_limits.max_image_height = Some(ceiling);
    image_limits.max_alloc = Some(limits.max_decoded_bytes);
    let mut decoder = GifDecoder::new(Cursor::new(bytes))?;
    decoder.set_limits(image_limits)?;

    let mut frames = Vec::with_capacity(header.frame_count);
    for frame in decoder.into_frames() {
        let frame = frame?;
        let delay_centiseconds = delay_centiseconds(frame.delay());
        let accepted = frames
            .len()
            .checked_add(1)
            .ok_or(ContractError::ArithmeticOverflow)?;
        if accepted > MAX_ANIMATION_FRAMES {
            return Err(frame_limit_error());
        }
        require_budget(accepted, frame_bytes, limits)?;
        let source = DecodedSource::from_rgba(dimensions, frame.into_buffer().into_raw(), limits)?;
        frames.push(AnimationFrame {
            source: Arc::new(source),
            delay_centiseconds,
        });
    }

    let loop_count = match header.repeat {
        Repeat::Infinite => LoopCount::Infinite,
        Repeat::Finite(count) => LoopCount::Finite(count),
    };
    Ok(Some(DecodedAnimation {
        metadata: SourceMetadata::new(dimensions, 4)?,
        frames,
        loop_count,
    }))
}

struct GifHeader {
    width: u32,
    height: u32,
    frame_count: usize,
    repeat: Repeat,
}

/// Reads the logical screen, frame count and loop extension without decoding pixels.
///
/// Scanning stops once more than [`MAX_ANIMATION_FRAMES`] frames are seen.
fn scan_gif(bytes: &[u8]) -> Result<GifHeader, DecodeError> {
    let mut options = DecodeOptions::new();
    options.skip_frame_decoding(true);
    let mut decoder = options.read_info(Cursor::new(bytes)).map_err(gif_error)?;
    let width = u32::from(decoder.width());
    let height = u32::from(decoder.height());
    let mut frame_count = 0_usize;
    while decoder.next_frame_info().map_err(gif_error)?.is_some() {
        frame_count = frame_count
            .checked_add(1)
            .ok_or(ContractError::ArithmeticOverflow)?;
        if frame_count > MAX_ANIMATION_FRAMES {
            return Err(frame_limit_error());
        }
    }
    Ok(GifHeader {
        width,
        height,
        frame_count,
        repeat: decoder.repeat(),
    })
}

/// Checks `(accepted + 1)` canvases, one of which is the decoder working buffer.
fn require_budget(
    accepted: usize,
    frame_bytes: u64,
    limits: DecodeLimits,
) -> Result<(), DecodeError> {
    let canvases = u64::try_from(accepted)
        .map_err(|_| ContractError::ArithmeticOverflow)?
        .checked_add(1)
        .ok_or(ContractError::ArithmeticOverflow)?;
    let total = canvases
        .checked_mul(frame_bytes)
        .ok_or(ContractError::ArithmeticOverflow)?;
    if total > limits.max_decoded_bytes {
        return Err(DecodeError::Contract(ContractError::ResourceLimitExceeded));
    }
    Ok(())
}

fn frame_limit_error() -> DecodeError {
    DecodeError::ResourceLimit(format!(
        "GIF animation exceeds {MAX_ANIMATION_FRAMES} frames"
    ))
}

fn gif_error(error: gif::DecodingError) -> DecodeError {
    DecodeError::Image(ImageError::Decoding(DecodingError::new(
        ImageFormatHint::Exact(ImageFormat::Gif),
        error,
    )))
}

/// Converts a delay to centiseconds, rounding half up and saturating at `u16::MAX`.
fn delay_centiseconds(delay: Delay) -> u16 {
    let (numerator, denominator) = delay.numer_denom_ms();
    let denominator = u64::from(denominator).max(1);
    let centiseconds = u64::from(numerator).saturating_add(denominator.saturating_mul(5))
        / denominator.saturating_mul(10);
    let clamped = u16::try_from(centiseconds).unwrap_or(u16::MAX);
    if clamped <= 1 {
        DEFAULT_DELAY_CENTISECONDS
    } else {
        clamped
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use gif::{DisposalMethod, Encoder as RawEncoder, Frame as RawFrame, Repeat as RawRepeat};
    use image::{Delay, DynamicImage, ImageBuffer, ImageFormat, Rgba};
    use image_to_binary_core::ResourceLimits;

    use super::*;

    const PALETTE: [u8; 12] = [255, 0, 0, 0, 0, 255, 0, 255, 0, 0, 0, 0];
    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const TRANSPARENT_INDEX: u8 = 3;

    struct Spec {
        left: u16,
        top: u16,
        width: u16,
        height: u16,
        delay: u16,
        transparent: Option<u8>,
        indices: Vec<u8>,
    }

    fn full_frame(width: u16, height: u16, index: u8, delay: u16) -> Spec {
        Spec {
            left: 0,
            top: 0,
            width,
            height,
            delay,
            transparent: None,
            indices: vec![index; usize::from(width) * usize::from(height)],
        }
    }

    fn encode(width: u16, height: u16, repeat: Option<RawRepeat>, frames: &[Spec]) -> Vec<u8> {
        let mut output = Vec::new();
        {
            let mut encoder = RawEncoder::new(&mut output, width, height, &PALETTE).unwrap();
            if let Some(repeat) = repeat {
                encoder.set_repeat(repeat).unwrap();
            }
            for spec in frames {
                encoder
                    .write_frame(&RawFrame {
                        left: spec.left,
                        top: spec.top,
                        width: spec.width,
                        height: spec.height,
                        delay: spec.delay,
                        dispose: DisposalMethod::Keep,
                        transparent: spec.transparent,
                        needs_user_input: false,
                        interlaced: false,
                        palette: None,
                        buffer: Cow::Borrowed(&spec.indices),
                    })
                    .unwrap();
            }
        }
        output
    }

    fn pixels_of(animation: &DecodedAnimation, index: usize) -> &[u8] {
        animation.frames()[index].source().pixels()
    }

    fn solid(color: [u8; 4], count: usize) -> Vec<u8> {
        color.repeat(count)
    }

    #[test]
    fn multi_frame_gif_yields_every_frame_on_the_canvas() {
        let bytes = encode(
            2,
            2,
            None,
            &[
                full_frame(2, 2, 0, 5),
                full_frame(2, 2, 1, 5),
                full_frame(2, 2, 2, 5),
            ],
        );
        let animation = decode_animation(&bytes, DecodeLimits::default())
            .unwrap()
            .unwrap();
        assert_eq!(animation.frames().len(), 3);
        assert_eq!(
            animation.metadata().dimensions(),
            Dimensions::new(2, 2).unwrap()
        );
        assert_eq!(animation.metadata().channels(), 4);
        assert_eq!(pixels_of(&animation, 0), solid(RED, 4));
        assert_eq!(pixels_of(&animation, 1), solid(BLUE, 4));
        assert_eq!(pixels_of(&animation, 2), solid(GREEN, 4));
        assert!(animation
            .frames()
            .iter()
            .all(|frame| frame.delay_centiseconds() == 5));
    }

    #[test]
    fn partial_frame_composites_with_offset_and_transparency() {
        let bytes = encode(
            3,
            1,
            None,
            &[
                full_frame(3, 1, 0, 10),
                Spec {
                    left: 1,
                    top: 0,
                    width: 2,
                    height: 1,
                    delay: 10,
                    transparent: Some(TRANSPARENT_INDEX),
                    indices: vec![TRANSPARENT_INDEX, 1],
                },
            ],
        );
        let animation = decode_animation(&bytes, DecodeLimits::default())
            .unwrap()
            .unwrap();
        let expected: Vec<u8> = [RED, RED, BLUE].concat();
        assert_eq!(pixels_of(&animation, 1), expected);
    }

    #[test]
    fn delays_round_to_centiseconds_and_normalise_short_values() {
        assert_eq!(delay_centiseconds(Delay::from_numer_denom_ms(0, 1)), 10);
        assert_eq!(delay_centiseconds(Delay::from_numer_denom_ms(10, 1)), 10);
        assert_eq!(delay_centiseconds(Delay::from_numer_denom_ms(14, 1)), 10);
        assert_eq!(delay_centiseconds(Delay::from_numer_denom_ms(15, 1)), 2);
        assert_eq!(delay_centiseconds(Delay::from_numer_denom_ms(25, 1)), 3);
        assert_eq!(delay_centiseconds(Delay::from_numer_denom_ms(1234, 10)), 12);
        assert_eq!(delay_centiseconds(Delay::from_numer_denom_ms(1255, 10)), 13);
        assert_eq!(delay_centiseconds(Delay::from_numer_denom_ms(1255, 1)), 126);
        assert_eq!(delay_centiseconds(Delay::from_numer_denom_ms(1500, 1)), 150);
        assert_eq!(
            delay_centiseconds(Delay::from_numer_denom_ms(u32::MAX, 1)),
            u16::MAX
        );
    }

    #[test]
    fn stored_zero_and_one_centisecond_frames_normalise_to_ten() {
        let bytes = encode(
            1,
            1,
            None,
            &[
                full_frame(1, 1, 0, 0),
                full_frame(1, 1, 1, 1),
                full_frame(1, 1, 2, 7),
            ],
        );
        let animation = decode_animation(&bytes, DecodeLimits::default())
            .unwrap()
            .unwrap();
        let delays: Vec<u16> = animation
            .frames()
            .iter()
            .map(AnimationFrame::delay_centiseconds)
            .collect();
        assert_eq!(delays, [10, 10, 7]);
    }

    #[test]
    fn loop_count_distinguishes_finite_infinite_and_absent() {
        let frames = [full_frame(1, 1, 0, 1), full_frame(1, 1, 1, 1)];
        let finite = encode(1, 1, Some(RawRepeat::Finite(3)), &frames);
        let infinite = encode(1, 1, Some(RawRepeat::Infinite), &frames);
        let absent = encode(1, 1, None, &frames);
        let limits = DecodeLimits::default();
        let loop_of = |bytes: &[u8]| {
            decode_animation(bytes, limits)
                .unwrap()
                .unwrap()
                .loop_count()
        };
        assert_eq!(loop_of(&finite), LoopCount::Finite(3));
        assert_eq!(loop_of(&infinite), LoopCount::Infinite);
        assert_eq!(loop_of(&absent), LoopCount::Finite(0));
    }

    #[test]
    fn frame_count_limit_accepts_one_thousand_and_rejects_one_thousand_one() {
        let at_limit: Vec<Spec> = (0..MAX_ANIMATION_FRAMES)
            .map(|_| full_frame(1, 1, 0, 1))
            .collect();
        let bytes = encode(1, 1, None, &at_limit);
        let animation = decode_animation(&bytes, DecodeLimits::default())
            .unwrap()
            .unwrap();
        assert_eq!(animation.frames().len(), MAX_ANIMATION_FRAMES);

        let over_limit: Vec<Spec> = (0..=MAX_ANIMATION_FRAMES)
            .map(|_| full_frame(1, 1, 0, 1))
            .collect();
        let bytes = encode(1, 1, None, &over_limit);
        assert!(matches!(
            decode_animation(&bytes, DecodeLimits::default()),
            Err(DecodeError::ResourceLimit(_))
        ));
    }

    #[test]
    fn byte_budget_counts_every_frame_plus_one_working_canvas() {
        let bytes = encode(
            2,
            1,
            None,
            &[
                full_frame(2, 1, 0, 1),
                full_frame(2, 1, 1, 1),
                full_frame(2, 1, 2, 1),
            ],
        );
        let exact = DecodeLimits::new(ResourceLimits::default(), 256 * 1024 * 1024, 32).unwrap();
        assert!(decode_animation(&bytes, exact).unwrap().is_some());
        let short = DecodeLimits::new(ResourceLimits::default(), 256 * 1024 * 1024, 31).unwrap();
        assert!(matches!(
            decode_animation(&bytes, short),
            Err(DecodeError::Contract(ContractError::ResourceLimitExceeded))
        ));
    }

    #[test]
    fn single_frame_gif_and_png_return_none() {
        let single = encode(2, 1, None, &[full_frame(2, 1, 0, 1)]);
        assert!(decode_animation(&single, DecodeLimits::default())
            .unwrap()
            .is_none());
        let mut png = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(ImageBuffer::from_pixel(1, 1, Rgba(RED)))
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        assert!(decode_animation(png.get_ref(), DecodeLimits::default())
            .unwrap()
            .is_none());
    }

    #[test]
    fn oversized_and_truncated_inputs_fail() {
        let limits = DecodeLimits::new(ResourceLimits::default(), 2, 1024).unwrap();
        assert!(matches!(
            decode_animation(&[0; 3], limits),
            Err(DecodeError::EncodedInputTooLarge)
        ));
        assert!(matches!(
            decode_animation(b"GIF89a", DecodeLimits::default()),
            Err(DecodeError::Image(_))
        ));
    }
}
