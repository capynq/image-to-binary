//! Bounded, frame-by-frame animated GIF encoding over the shared counting sink.

use std::borrow::Cow;
use std::io::Write;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use gif::{DisposalMethod, Frame, Repeat};
use image_to_binary_core::ContractError;

use crate::counting::{CountedWriter, WriteFlags};
use crate::ExportError;

/// Speed passed to `gif::Frame::from_rgba_speed` for source-colour frames. It is fixed so equal
/// frames always quantise to equal bytes; the crate's default of 10 trades palette quality for time.
pub const SOURCE_QUANTIZATION_SPEED: i32 = 10;

const MONOCHROME_ENTRIES: usize = 256;

/// Loop behaviour written as the NETSCAPE2.0 application extension.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GifRepeat {
    /// Repeats until playback stops.
    Infinite,
    /// Repeats this many times after the first play. `Finite(0)` plays once and writes no extension.
    Finite(u16),
}

/// How frame pixels become GIF palette indices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GifColors {
    /// Every frame indexes one global palette of up to 256 entries interpolated from
    /// `background` to `foreground`. Monochrome renders are blends of these two colours, so each
    /// pixel maps to its nearest entry. Equal endpoints give a single entry.
    Monochrome {
        /// Colour of coverage zero.
        background: [u8; 3],
        /// Colour of full coverage.
        foreground: [u8; 3],
    },
    /// Each frame carries its own palette from `gif::Frame::from_rgba_speed` at
    /// [`SOURCE_QUANTIZATION_SPEED`].
    Source,
}

/// Checked canvas size, loop behaviour and colour policy for one GIF stream.
///
/// Invariant: both dimensions lie in `1..=65535`, the GIF logical screen limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GifExportPlan {
    width: u16,
    height: u16,
    max_encoded_bytes: u64,
    repeat: GifRepeat,
    colors: GifColors,
}

impl GifExportPlan {
    /// Creates a plan for a `width` by `height` canvas with a bounded encoded output.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidDimensions`] for a zero dimension or one above 65535,
    /// and [`ContractError::InvalidSettings`] for a zero output ceiling.
    pub fn new(
        width: u32,
        height: u32,
        max_encoded_bytes: u64,
        repeat: GifRepeat,
        colors: GifColors,
    ) -> Result<Self, ExportError> {
        let width = screen_dimension(width)?;
        let height = screen_dimension(height)?;
        if max_encoded_bytes == 0 {
            return Err(ContractError::InvalidSettings("output limit").into());
        }
        Ok(Self {
            width,
            height,
            max_encoded_bytes,
            repeat,
            colors,
        })
    }

    /// Starts a stream by writing the header, global palette and loop extension to `sink`.
    /// Frames follow through [`GifStreamEncoder::write_frame`]; [`GifStreamEncoder::finish`]
    /// writes the trailer.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::ResourceLimitExceeded`] when the header exceeds the output
    /// ceiling or the sink's storage ceiling, and [`ExportError::Gif`] for other sink failures.
    pub fn stream_encoder<W: Write + 'static>(
        self,
        sink: W,
    ) -> Result<GifStreamEncoder<W>, ExportError> {
        let flags = Arc::new(WriteFlags::default());
        let counted = CountedWriter::new(sink, self.max_encoded_bytes, Arc::clone(&flags));
        let (mut encoder, colors) = match self.colors {
            GifColors::Monochrome {
                background,
                foreground,
            } => {
                let palette = MonochromePalette::new(background, foreground)?;
                let encoder = gif::Encoder::new(counted, self.width, self.height, palette.table())
                    .map_err(|error| gif_error(&flags, error))?;
                (encoder, ColorMapping::Monochrome(palette))
            }
            GifColors::Source => {
                let encoder = gif::Encoder::new(counted, self.width, self.height, &[])
                    .map_err(|error| gif_error(&flags, error))?;
                (encoder, ColorMapping::Source)
            }
        };
        match self.repeat {
            GifRepeat::Infinite => encoder.set_repeat(Repeat::Infinite),
            GifRepeat::Finite(0) => Ok(()),
            GifRepeat::Finite(count) => encoder.set_repeat(Repeat::Finite(count)),
        }
        .map_err(|error| gif_error(&flags, error))?;
        let indices = match colors {
            ColorMapping::Monochrome(_) => {
                let mut indices = Vec::new();
                indices
                    .try_reserve_exact(pixel_count(self.width, self.height)?)
                    .map_err(|_| ContractError::ResourceLimitExceeded)?;
                indices
            }
            ColorMapping::Source => Vec::new(),
        };
        Ok(GifStreamEncoder {
            encoder,
            flags,
            width: self.width,
            height: self.height,
            colors,
            indices,
        })
    }

    /// Returns the canvas width in pixels.
    #[must_use]
    pub const fn width(self) -> u16 {
        self.width
    }

    /// Returns the canvas height in pixels.
    #[must_use]
    pub const fn height(self) -> u16 {
        self.height
    }
}

fn screen_dimension(value: u32) -> Result<u16, ExportError> {
    match u16::try_from(value) {
        Ok(dimension) if dimension > 0 => Ok(dimension),
        _ => Err(ContractError::InvalidDimensions.into()),
    }
}

fn pixel_count(width: u16, height: u16) -> Result<usize, ExportError> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(ContractError::ArithmeticOverflow)?;
    Ok(usize::try_from(pixels).map_err(|_| ContractError::ArithmeticOverflow)?)
}

fn gif_error(flags: &WriteFlags, error: gif::EncodingError) -> ExportError {
    if flags.resource_limit.load(Ordering::Acquire) {
        ExportError::Contract(ContractError::ResourceLimitExceeded)
    } else {
        ExportError::Gif(error)
    }
}

#[derive(Debug)]
enum ColorMapping {
    Monochrome(MonochromePalette),
    Source,
}

/// Incremental GIF encoder that owns its sink and writes one full-canvas frame per call.
///
/// Invariants: every frame is exactly `width * height` RGBA8 pixels; the canvas never changes
/// between frames; frames use disposal `Keep`, so each one replaces the whole canvas. Dropping an
/// unfinished encoder discards the stream; callers must also discard bytes already written.
pub struct GifStreamEncoder<W: Write + 'static> {
    encoder: gif::Encoder<CountedWriter<W>>,
    flags: Arc<WriteFlags>,
    width: u16,
    height: u16,
    colors: ColorMapping,
    indices: Vec<u8>,
}

impl<W: Write + 'static> GifStreamEncoder<W> {
    /// Encodes one frame of opaque RGBA8 pixels shown for `delay_centiseconds`.
    ///
    /// Source-colour frames are quantised in place, so `rgba` may be rewritten; monochrome frames
    /// are left unchanged. Alpha is ignored.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidDimensions`] when `rgba` is not exactly one canvas,
    /// [`ContractError::ResourceLimitExceeded`] when the output or sink ceiling is reached, and
    /// [`ExportError::Gif`] for other encoder or sink failures.
    pub fn write_frame(
        &mut self,
        rgba: &mut [u8],
        delay_centiseconds: u16,
    ) -> Result<(), ExportError> {
        let expected = pixel_count(self.width, self.height)?
            .checked_mul(4)
            .ok_or(ContractError::ArithmeticOverflow)?;
        if rgba.len() != expected {
            return Err(ContractError::InvalidDimensions.into());
        }
        let Self {
            encoder,
            flags,
            width,
            height,
            colors,
            indices,
        } = self;
        let result = match colors {
            ColorMapping::Monochrome(palette) => {
                indices.clear();
                for pixel in rgba.chunks_exact(4) {
                    indices.push(palette.index_of([pixel[0], pixel[1], pixel[2]])?);
                }
                let frame = Frame {
                    width: *width,
                    height: *height,
                    delay: delay_centiseconds,
                    dispose: DisposalMethod::Keep,
                    buffer: Cow::Borrowed(indices.as_slice()),
                    ..Frame::default()
                };
                encoder.write_frame(&frame)
            }
            ColorMapping::Source => {
                let mut frame =
                    Frame::from_rgba_speed(*width, *height, rgba, SOURCE_QUANTIZATION_SPEED);
                frame.delay = delay_centiseconds;
                frame.dispose = DisposalMethod::Keep;
                encoder.write_frame(&frame)
            }
        };
        result.map_err(|error| gif_error(flags, error))
    }

    /// Writes the trailer and flushes the sink, so a flush failure cannot look like success.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::ResourceLimitExceeded`] when the trailer or flush hits the
    /// output or sink ceiling, and [`ExportError::Gif`] for other failures.
    pub fn finish(self) -> Result<(), ExportError> {
        let Self { encoder, flags, .. } = self;
        let mut counted = encoder
            .into_inner()
            .map_err(|error| gif_error(&flags, error))?;
        counted
            .flush()
            .map_err(|error| gif_error(&flags, gif::EncodingError::Io(error)))?;
        drop(counted);
        if flags.failed.load(Ordering::Acquire) {
            return Err(gif_error(
                &flags,
                gif::EncodingError::Io(std::io::Error::other(
                    "GIF sink failed while releasing the stream",
                )),
            ));
        }
        Ok(())
    }
}

/// The monochrome global palette and its projection rule. Both endpoints are exact entries, and
/// every other colour maps to the entry nearest its projection on the background-to-foreground line.
#[derive(Clone, Debug, Eq, PartialEq)]
struct MonochromePalette {
    background: [i64; 3],
    span: [i64; 3],
    span_squared: i64,
    steps: i64,
    table: Vec<u8>,
}

impl MonochromePalette {
    fn new(background: [u8; 3], foreground: [u8; 3]) -> Result<Self, ContractError> {
        let background = background.map(i64::from);
        let foreground = foreground.map(i64::from);
        let mut span = [0_i64; 3];
        for (channel, value) in span.iter_mut().enumerate() {
            *value = foreground[channel] - background[channel];
        }
        let span_squared = span.iter().try_fold(0_i64, |sum, value| {
            value
                .checked_mul(*value)
                .and_then(|square| sum.checked_add(square))
        });
        let span_squared = span_squared.ok_or(ContractError::ArithmeticOverflow)?;
        let entries = if span_squared == 0 {
            1
        } else {
            MONOCHROME_ENTRIES
        };
        let steps = i64::try_from(entries.saturating_sub(1))
            .map_err(|_| ContractError::ArithmeticOverflow)?;
        let mut table = Vec::new();
        table
            .try_reserve_exact(entries * 3)
            .map_err(|_| ContractError::ResourceLimitExceeded)?;
        for index in 0..entries {
            let index = i64::try_from(index).map_err(|_| ContractError::ArithmeticOverflow)?;
            for channel in 0..3 {
                let offset = if steps == 0 {
                    0
                } else {
                    let scaled = span[channel]
                        .checked_mul(index)
                        .ok_or(ContractError::ArithmeticOverflow)?;
                    round_div(scaled, steps).ok_or(ContractError::ArithmeticOverflow)?
                };
                let value = background[channel]
                    .checked_add(offset)
                    .ok_or(ContractError::ArithmeticOverflow)?;
                table.push(u8::try_from(value).map_err(|_| ContractError::ArithmeticOverflow)?);
            }
        }
        Ok(Self {
            background,
            span,
            span_squared,
            steps,
            table,
        })
    }

    fn table(&self) -> &[u8] {
        &self.table
    }

    fn index_of(&self, pixel: [u8; 3]) -> Result<u8, ContractError> {
        if self.steps == 0 {
            return Ok(0);
        }
        let mut projection = 0_i64;
        for ((value, background), span) in pixel.iter().zip(self.background).zip(self.span) {
            let offset = i64::from(*value)
                .checked_sub(background)
                .ok_or(ContractError::ArithmeticOverflow)?;
            let term = offset
                .checked_mul(span)
                .ok_or(ContractError::ArithmeticOverflow)?;
            projection = projection
                .checked_add(term)
                .ok_or(ContractError::ArithmeticOverflow)?;
        }
        let clamped = projection.clamp(0, self.span_squared);
        let scaled = clamped
            .checked_mul(self.steps)
            .ok_or(ContractError::ArithmeticOverflow)?;
        let index =
            round_div(scaled, self.span_squared).ok_or(ContractError::ArithmeticOverflow)?;
        u8::try_from(index).map_err(|_| ContractError::ArithmeticOverflow)
    }
}

/// Rounds `numerator / denominator` half up for a positive denominator, on integers only.
fn round_div(numerator: i64, denominator: i64) -> Option<i64> {
    let doubled = numerator.checked_mul(2)?.checked_add(denominator)?;
    doubled.checked_div_euclid(denominator.checked_mul(2)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_chunk_channel;
    use std::io::Cursor;
    use std::sync::{Arc, Mutex};

    const LIMIT: u64 = 1 << 20;

    fn monochrome(background: [u8; 3], foreground: [u8; 3]) -> GifColors {
        GifColors::Monochrome {
            background,
            foreground,
        }
    }

    fn plan(width: u32, height: u32, repeat: GifRepeat, colors: GifColors) -> GifExportPlan {
        GifExportPlan::new(width, height, LIMIT, repeat, colors).unwrap()
    }

    fn encode(plan: GifExportPlan, frames: &[(Vec<u8>, u16)]) -> Result<Vec<u8>, ExportError> {
        let target = Arc::new(Mutex::new(Vec::new()));
        let mut encoder = plan.stream_encoder(SharedSink(Arc::clone(&target)))?;
        for (pixels, delay) in frames {
            encoder.write_frame(&mut pixels.clone(), *delay)?;
        }
        encoder.finish()?;
        let bytes = target.lock().unwrap().clone();
        Ok(bytes)
    }

    #[derive(Debug)]
    struct SharedSink(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedSink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct Decoded {
        width: u16,
        height: u16,
        repeat: Repeat,
        frames: Vec<(Vec<u8>, u16)>,
    }

    fn decode(bytes: &[u8]) -> Decoded {
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(Cursor::new(bytes.to_vec())).unwrap();
        let (width, height, repeat) = (decoder.width(), decoder.height(), decoder.repeat());
        let mut frames = Vec::new();
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            frames.push((frame.buffer.to_vec(), frame.delay));
        }
        Decoded {
            width,
            height,
            repeat,
            frames,
        }
    }

    fn netscape_block(repeat: Repeat) -> Vec<u8> {
        let mut block = vec![0x21, 0xFF, 0x0B];
        block.extend_from_slice(b"NETSCAPE2.0");
        block.push(0x03);
        block.push(0x01);
        match repeat {
            Repeat::Infinite => block.extend_from_slice(&[0, 0]),
            Repeat::Finite(count) => block.extend_from_slice(&count.to_le_bytes()),
        }
        block.push(0x00);
        block
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }

    fn two_tone_frames() -> Vec<(Vec<u8>, u16)> {
        let black = [0, 0, 0, 255];
        let green = [0, 255, 0, 255];
        let grey = [0, 128, 0, 255];
        let frame =
            |colours: [[u8; 4]; 3]| -> Vec<u8> { colours.iter().flatten().copied().collect() };
        vec![
            (frame([black, green, grey]), 5),
            (frame([green, black, black]), 10),
            (frame([black, black, green]), 50),
        ]
    }

    #[test]
    fn monochrome_round_trip_keeps_frame_count_delays_and_exact_endpoints() {
        let frames = two_tone_frames();
        let bytes = encode(
            plan(
                3,
                1,
                GifRepeat::Infinite,
                monochrome([0, 0, 0], [0, 255, 0]),
            ),
            &frames,
        )
        .unwrap();
        assert!(bytes.starts_with(b"GIF89a"));
        let decoded = decode(&bytes);
        assert_eq!((decoded.width, decoded.height), (3, 1));
        assert_eq!(decoded.repeat, Repeat::Infinite);
        let delays: Vec<u16> = decoded.frames.iter().map(|(_, delay)| *delay).collect();
        assert_eq!(delays, [5, 10, 50]);
        assert_eq!(decoded.frames[0].0[..4], [0, 0, 0, 255]);
        assert_eq!(decoded.frames[0].0[4..8], [0, 255, 0, 255]);
        assert_eq!(decoded.frames[1].0[..4], [0, 255, 0, 255]);
        assert_eq!(decoded.frames[1].0[4..8], [0, 0, 0, 255]);
        let mid = &decoded.frames[0].0[8..12];
        assert_eq!(mid[0], 0);
        assert!(mid[1].abs_diff(128) <= 1);
        assert_eq!(mid[3], 255);
    }

    #[test]
    fn loop_extension_is_written_for_each_repeat_case() {
        let frames = two_tone_frames();
        let colours = monochrome([0, 0, 0], [255, 255, 255]);
        for repeat in [Repeat::Infinite, Repeat::Finite(3), Repeat::Finite(1)] {
            let loop_repeat = match repeat {
                Repeat::Infinite => GifRepeat::Infinite,
                Repeat::Finite(count) => GifRepeat::Finite(count),
            };
            let bytes = encode(plan(3, 1, loop_repeat, colours), &frames).unwrap();
            assert!(contains(&bytes, &netscape_block(repeat)));
            assert_eq!(decode(&bytes).repeat, repeat);
        }
        let once = encode(plan(3, 1, GifRepeat::Finite(0), colours), &frames).unwrap();
        assert!(!contains(&once, b"NETSCAPE2.0"));
        assert_eq!(decode(&once).repeat, Repeat::Finite(0));
        assert_eq!(decode(&once).frames.len(), 3);
    }

    #[test]
    fn equal_endpoints_use_a_single_entry_palette() {
        let palette = MonochromePalette::new([9, 8, 7], [9, 8, 7]).unwrap();
        assert_eq!(palette.table(), &[9, 8, 7]);
        assert_eq!(palette.index_of([200, 1, 3]).unwrap(), 0);
        let bytes = encode(
            plan(2, 1, GifRepeat::Finite(0), monochrome([9, 8, 7], [9, 8, 7])),
            &[(vec![9, 8, 7, 255, 9, 8, 7, 255], 10)],
        )
        .unwrap();
        assert_eq!(decode(&bytes).frames[0].0, [9, 8, 7, 255, 9, 8, 7, 255]);
    }

    #[test]
    fn palette_interpolates_and_projects_to_the_nearest_entry() {
        let palette = MonochromePalette::new([0, 0, 0], [255, 128, 0]).unwrap();
        assert_eq!(palette.table().len(), 256 * 3);
        assert_eq!(&palette.table()[..3], &[0, 0, 0]);
        assert_eq!(&palette.table()[255 * 3..], &[255, 128, 0]);
        assert_eq!(palette.index_of([0, 0, 0]).unwrap(), 0);
        assert_eq!(palette.index_of([255, 128, 0]).unwrap(), 255);
        assert_eq!(palette.index_of([128, 64, 0]).unwrap(), 128);
        assert_eq!(palette.index_of([255, 0, 255]).unwrap(), 204);
        assert_eq!(palette.index_of([0, 255, 0]).unwrap(), 102);
        let mid_index = palette.index_of([128, 64, 0]).unwrap();
        let entry = &palette.table()[usize::from(mid_index) * 3..][..3];
        assert_eq!(entry, [128, 64, 0]);
    }

    #[test]
    fn round_div_rounds_half_up_on_integers() {
        assert_eq!(round_div(5, 2), Some(3));
        assert_eq!(round_div(-1, 2), Some(0));
        assert_eq!(round_div(4, 3), Some(1));
        assert_eq!(round_div(5, 0), None);
    }

    #[test]
    fn source_colour_round_trip_is_deterministic_and_keeps_timing() {
        let frames: Vec<(Vec<u8>, u16)> = (0..4_u8)
            .map(|shade| {
                let pixels = (0..12_u8)
                    .flat_map(|pixel| {
                        [
                            shade.wrapping_mul(40).wrapping_add(pixel * 9),
                            pixel * 17,
                            shade * 60,
                            255,
                        ]
                    })
                    .collect();
                (pixels, u16::from(shade) * 10 + 10)
            })
            .collect();
        let build = || encode(plan(4, 3, GifRepeat::Infinite, GifColors::Source), &frames).unwrap();
        let first = build();
        assert_eq!(first, build());
        let decoded = decode(&first);
        assert_eq!((decoded.width, decoded.height), (4, 3));
        let delays: Vec<u16> = decoded.frames.iter().map(|(_, delay)| *delay).collect();
        assert_eq!(delays, [10, 20, 30, 40]);
    }

    #[test]
    fn monochrome_output_is_byte_identical_across_encodes() {
        let frames = two_tone_frames();
        let colours = monochrome([10, 20, 30], [200, 210, 220]);
        let first = encode(plan(3, 1, GifRepeat::Finite(2), colours), &frames).unwrap();
        let second = encode(plan(3, 1, GifRepeat::Finite(2), colours), &frames).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn browser_chunks_and_direct_sink_produce_identical_bytes() {
        let frames = two_tone_frames();
        let colours = monochrome([0, 0, 0], [255, 255, 255]);
        let direct = encode(plan(3, 1, GifRepeat::Infinite, colours), &frames).unwrap();
        let (writer, drain) = browser_chunk_channel(7, LIMIT).unwrap();
        let mut encoder = plan(3, 1, GifRepeat::Infinite, colours)
            .stream_encoder(writer)
            .unwrap();
        let mut streamed = Vec::new();
        for (pixels, delay) in &frames {
            encoder.write_frame(&mut pixels.clone(), *delay).unwrap();
            for chunk in drain.drain().unwrap() {
                streamed.extend(chunk);
            }
        }
        encoder.finish().unwrap();
        for chunk in drain.take_remaining().unwrap() {
            streamed.extend(chunk);
        }
        assert_eq!(streamed, direct);
    }

    #[test]
    fn dimensions_above_the_gif_limit_are_typed_errors() {
        assert!(matches!(
            GifExportPlan::new(65_536, 1, LIMIT, GifRepeat::Infinite, GifColors::Source),
            Err(ExportError::Contract(ContractError::InvalidDimensions))
        ));
        assert!(matches!(
            GifExportPlan::new(1, 65_536, LIMIT, GifRepeat::Infinite, GifColors::Source),
            Err(ExportError::Contract(ContractError::InvalidDimensions))
        ));
        assert!(matches!(
            GifExportPlan::new(0, 1, LIMIT, GifRepeat::Infinite, GifColors::Source),
            Err(ExportError::Contract(ContractError::InvalidDimensions))
        ));
        let largest = GifExportPlan::new(65_535, 1, LIMIT, GifRepeat::Infinite, GifColors::Source);
        assert_eq!(largest.unwrap().width(), 65_535);
        assert!(matches!(
            GifExportPlan::new(1, 1, 0, GifRepeat::Infinite, GifColors::Source),
            Err(ExportError::Contract(ContractError::InvalidSettings(_)))
        ));
    }

    #[test]
    fn wrong_frame_length_is_rejected_before_any_write() {
        let mut encoder = plan(2, 2, GifRepeat::Infinite, GifColors::Source)
            .stream_encoder(Vec::new())
            .unwrap();
        assert!(matches!(
            encoder.write_frame(&mut [0; 12], 10),
            Err(ExportError::Contract(ContractError::InvalidDimensions))
        ));
    }

    #[test]
    fn output_limit_is_a_typed_resource_error() {
        let frames = two_tone_frames();
        let small = GifExportPlan::new(
            3,
            1,
            40,
            GifRepeat::Infinite,
            monochrome([0, 0, 0], [255, 255, 255]),
        )
        .unwrap();
        let result = encode(small, &frames);
        assert!(matches!(
            result,
            Err(ExportError::Contract(ContractError::ResourceLimitExceeded))
        ));
    }

    #[derive(Debug)]
    struct FailingSink;

    impl Write for FailingSink {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("disk full"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn sink_failures_are_reported_as_gif_errors() {
        let result = plan(3, 1, GifRepeat::Infinite, GifColors::Source).stream_encoder(FailingSink);
        assert!(matches!(result, Err(ExportError::Gif(_))));
    }
}
