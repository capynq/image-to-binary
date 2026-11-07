//! Format, orientation, depth, frame, and limit coverage for every decode adapter.
//!
//! Common-codec fixtures are generated inside each test. HEIF and AVIF fixtures
//! are checked in under `fixtures/heif` and described in its PROVENANCE.md.

use std::io::{Cursor, Seek, Write};

use image::codecs::bmp::BmpEncoder;
use image::codecs::gif::{GifDecoder, GifEncoder};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;
use image::{
    AnimationDecoder, DynamicImage, ExtendedColorType, Frame, ImageBuffer, ImageEncoder,
    ImageError, Rgb, Rgba,
};
use image_to_binary_core::{ContractError, Dimensions, ResourceLimits};
use image_to_binary_processing::{decode, DecodeError, DecodeLimits, DecodedSource};
use tiff::decoder::Decoder as TiffDecoder;
use tiff::encoder::{colortype, TiffEncoder};
use tiff::tags::Tag;

type TestResult<T> = Result<T, Box<dyn std::error::Error>>;

const WIDTH: u32 = 24;
const HEIGHT: u32 = 16;
const BLOCK: u32 = 8;
const BLOCK_COLOURS: [[u8; 3]; 6] = [
    [230, 20, 20],
    [20, 230, 20],
    [20, 20, 230],
    [230, 230, 20],
    [230, 20, 230],
    [20, 230, 230],
];
const BLOCK_GREYS: [u8; 6] = [16, 64, 96, 144, 200, 240];
const PARTIAL_ALPHAS: [u8; 6] = [255, 220, 180, 140, 100, 60];
const BINARY_ALPHAS: [u8; 6] = [255, 0, 255, 0, 255, 0];
const LOSSY_TOLERANCE: u8 = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Codec {
    Jpeg,
    Png,
    WebP,
    Bmp,
    Tiff,
    Gif,
}

const COMMON: [Codec; 6] = [
    Codec::Jpeg,
    Codec::Png,
    Codec::WebP,
    Codec::Bmp,
    Codec::Tiff,
    Codec::Gif,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Layout {
    Grey,
    Rgb,
    Rgba,
}

impl Layout {
    const fn color(self) -> ExtendedColorType {
        match self {
            Self::Grey => ExtendedColorType::L8,
            Self::Rgb => ExtendedColorType::Rgb8,
            Self::Rgba => ExtendedColorType::Rgba8,
        }
    }
}

fn tolerance(codec: Codec) -> u8 {
    if codec == Codec::Jpeg {
        LOSSY_TOLERANCE
    } else {
        0
    }
}

fn block_at(x: u32, y: u32) -> usize {
    usize::try_from((y / BLOCK) * (WIDTH / BLOCK) + x / BLOCK).unwrap()
}

fn block_pixels(layout: Layout, alphas: [u8; 6]) -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let block = block_at(x, y);
            match layout {
                Layout::Grey => pixels.push(BLOCK_GREYS[block]),
                Layout::Rgb => pixels.extend_from_slice(&BLOCK_COLOURS[block]),
                Layout::Rgba => {
                    pixels.extend_from_slice(&BLOCK_COLOURS[block]);
                    pixels.push(alphas[block]);
                }
            }
        }
    }
    pixels
}

fn as_rgba(layout: Layout, pixels: &[u8]) -> Vec<u8> {
    match layout {
        Layout::Grey => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        Layout::Rgb => pixels
            .chunks_exact(3)
            .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255])
            .collect(),
        Layout::Rgba => pixels.to_vec(),
    }
}

fn exif_block(orientation: u8) -> Vec<u8> {
    let mut block = b"II*\0".to_vec();
    block.extend_from_slice(&8_u32.to_le_bytes());
    block.extend_from_slice(&1_u16.to_le_bytes());
    block.extend_from_slice(&0x0112_u16.to_le_bytes());
    block.extend_from_slice(&3_u16.to_le_bytes());
    block.extend_from_slice(&1_u32.to_le_bytes());
    block.extend_from_slice(&u16::from(orientation).to_le_bytes());
    block.extend_from_slice(&[0, 0]);
    block.extend_from_slice(&0_u32.to_le_bytes());
    block
}

fn write_tiff_page<W: Write + Seek, C: colortype::ColorType<Inner = u8>>(
    encoder: &mut TiffEncoder<W>,
    width: u32,
    height: u32,
    pixels: &[u8],
    orientation: Option<u8>,
) -> TestResult<()> {
    let mut image = encoder.new_image::<C>(width, height)?;
    if let Some(orientation) = orientation {
        image
            .encoder()
            .write_tag(Tag::Orientation, u16::from(orientation))?;
    }
    image.write_data(pixels)?;
    Ok(())
}

fn encode_gif(frames: Vec<(u32, u32, Vec<u8>)>) -> TestResult<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    let frames = frames.into_iter().map(|(width, height, rgba)| {
        let buffer = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, rgba).unwrap();
        Frame::new(buffer)
    });
    GifEncoder::new(&mut output).encode_frames(frames)?;
    Ok(output.into_inner())
}

fn encode(
    codec: Codec,
    layout: Layout,
    width: u32,
    height: u32,
    pixels: &[u8],
    orientation: Option<u8>,
) -> TestResult<Vec<u8>> {
    let mut output = Vec::new();
    match codec {
        Codec::Jpeg => {
            let mut encoder = JpegEncoder::new_with_quality(&mut output, 100);
            if let Some(orientation) = orientation {
                encoder.set_exif_metadata(exif_block(orientation))?;
            }
            encoder.write_image(pixels, width, height, layout.color())?;
        }
        Codec::Png => {
            let mut encoder = PngEncoder::new(&mut output);
            if let Some(orientation) = orientation {
                encoder.set_exif_metadata(exif_block(orientation))?;
            }
            encoder.write_image(pixels, width, height, layout.color())?;
        }
        Codec::WebP => {
            let mut encoder = WebPEncoder::new_lossless(&mut output);
            if let Some(orientation) = orientation {
                encoder.set_exif_metadata(exif_block(orientation))?;
            }
            encoder.write_image(pixels, width, height, layout.color())?;
        }
        Codec::Bmp => {
            BmpEncoder::new(&mut output).write_image(pixels, width, height, layout.color())?;
        }
        Codec::Tiff => {
            let mut encoder = TiffEncoder::new(Cursor::new(&mut output))?;
            match layout {
                Layout::Grey => {
                    write_tiff_page::<_, colortype::Gray8>(
                        &mut encoder,
                        width,
                        height,
                        pixels,
                        orientation,
                    )?;
                }
                Layout::Rgb => {
                    write_tiff_page::<_, colortype::RGB8>(
                        &mut encoder,
                        width,
                        height,
                        pixels,
                        orientation,
                    )?;
                }
                Layout::Rgba => {
                    write_tiff_page::<_, colortype::RGBA8>(
                        &mut encoder,
                        width,
                        height,
                        pixels,
                        orientation,
                    )?;
                }
            }
        }
        Codec::Gif => {
            let rgba = as_rgba(layout, pixels);
            return encode_gif(vec![(width, height, rgba)]);
        }
    }
    Ok(output)
}

fn decode_default(bytes: &[u8]) -> DecodedSource {
    decode(bytes, DecodeLimits::default()).unwrap()
}

/// Asserts that a crafted header passes the limit, then fails in the codec's own way.
///
/// The header declares more pixels than the payload holds, so the outcome after
/// the limit check is codec-specific. JPEG and GIF complete with the declared
/// dimensions. PNG and WebP report malformed data, and BMP and TIFF report an
/// unexpected end of input. Any other outcome, including a resource-limit error,
/// fails the assertion.
fn assert_limit_passed_then_truncated(
    codec: Codec,
    bytes: &[u8],
    limits: DecodeLimits,
    width: u32,
    height: u32,
) {
    match (codec, decode(bytes, limits)) {
        (Codec::Jpeg | Codec::Gif, Ok(decoded)) => assert_dimensions(&decoded, width, height),
        (Codec::Png | Codec::WebP, Err(DecodeError::Image(ImageError::Decoding(_))))
        | (Codec::Bmp | Codec::Tiff, Err(DecodeError::Image(ImageError::IoError(_)))) => {}
        (_, outcome) => panic!("{codec:?} {width}x{height}: unexpected outcome {outcome:?}"),
    }
}

fn assert_dimensions(decoded: &DecodedSource, width: u32, height: u32) {
    assert_eq!(
        decoded.metadata().dimensions(),
        Dimensions::new(width, height).unwrap()
    );
}

/// Compares alpha exactly and colour only where the expected alpha is nonzero.
///
/// Fully transparent pixels have no defined colour, so their RGB may differ.
fn assert_alpha_and_visible_colour(decoded: &DecodedSource, rgba_source: &[u8], codec: Codec) {
    let expected = as_rgba(Layout::Rgba, rgba_source);
    let label = format!("{codec:?} alpha");
    for (index, (actual, expected)) in decoded
        .pixels()
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .enumerate()
    {
        assert_eq!(actual[3], expected[3], "{label}: alpha at pixel {index}");
        if expected[3] != 0 {
            assert_rgba_within(&actual[..3], &expected[..3], tolerance(codec), &label);
        }
    }
}

fn assert_rgba_within(actual: &[u8], expected: &[u8], tolerance: u8, label: &str) {
    assert_eq!(actual.len(), expected.len(), "{label}: pixel byte length");
    let worst = actual
        .iter()
        .zip(expected)
        .map(|(actual, expected)| actual.abs_diff(*expected))
        .max()
        .unwrap_or(0);
    assert!(
        worst <= tolerance,
        "{label}: worst channel difference {worst} exceeds tolerance {tolerance}"
    );
}

/// Applies the EXIF orientation transform to tightly packed `channels`-wide pixels.
///
/// Values 5 to 8 swap width and height. Mapping follows the EXIF definition:
/// 6 rotates clockwise by 90 degrees and 8 rotates anticlockwise by 90 degrees.
fn orient(
    pixels: &[u8],
    width: u32,
    height: u32,
    orientation: u8,
    channels: usize,
) -> (u32, u32, Vec<u8>) {
    let w = usize::try_from(width).unwrap();
    let h = usize::try_from(height).unwrap();
    let (out_w, out_h) = if matches!(orientation, 5..=8) {
        (h, w)
    } else {
        (w, h)
    };
    let mut out = Vec::with_capacity(out_w * out_h * channels);
    for y in 0..out_h {
        for x in 0..out_w {
            let (sx, sy) = match orientation {
                1 => (x, y),
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (y, h - 1 - x),
                7 => (w - 1 - y, h - 1 - x),
                8 => (w - 1 - y, x),
                other => panic!("unsupported EXIF orientation {other}"),
            };
            let start = (sy * w + sx) * channels;
            out.extend_from_slice(&pixels[start..start + channels]);
        }
    }
    (
        u32::try_from(out_w).unwrap(),
        u32::try_from(out_h).unwrap(),
        out,
    )
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn with_png_dimensions(bytes: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut patched = bytes.to_vec();
    patched[16..20].copy_from_slice(&width.to_be_bytes());
    patched[20..24].copy_from_slice(&height.to_be_bytes());
    let crc = crc32(&patched[12..29]);
    patched[29..33].copy_from_slice(&crc.to_be_bytes());
    patched
}

fn with_jpeg_dimensions(bytes: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut patched = bytes.to_vec();
    let marker = patched
        .windows(2)
        .position(|pair| pair == [0xFF, 0xC0])
        .unwrap();
    let height = u16::try_from(height).unwrap();
    let width = u16::try_from(width).unwrap();
    patched[marker + 5..marker + 7].copy_from_slice(&height.to_be_bytes());
    patched[marker + 7..marker + 9].copy_from_slice(&width.to_be_bytes());
    patched
}

fn with_bmp_dimensions(bytes: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut patched = bytes.to_vec();
    patched[18..22].copy_from_slice(&i32::try_from(width).unwrap().to_le_bytes());
    patched[22..26].copy_from_slice(&i32::try_from(height).unwrap().to_le_bytes());
    patched
}

fn with_gif_dimensions(bytes: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut patched = bytes.to_vec();
    patched[6..8].copy_from_slice(&u16::try_from(width).unwrap().to_le_bytes());
    patched[8..10].copy_from_slice(&u16::try_from(height).unwrap().to_le_bytes());
    patched
}

fn with_tiff_dimensions(bytes: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut patched = bytes.to_vec();
    let ifd = usize::try_from(u32::from_le_bytes(bytes[4..8].try_into().unwrap())).unwrap();
    let count = usize::from(u16::from_le_bytes(bytes[ifd..ifd + 2].try_into().unwrap()));
    for index in 0..count {
        let entry = ifd + 2 + 12 * index;
        let tag = u16::from_le_bytes(bytes[entry..entry + 2].try_into().unwrap());
        let value = match tag {
            256 => width,
            257 => height,
            _ => continue,
        };
        let field = entry + 8;
        let kind = u16::from_le_bytes(bytes[entry + 2..entry + 4].try_into().unwrap());
        match kind {
            3 => patched[field..field + 2]
                .copy_from_slice(&u16::try_from(value).unwrap().to_le_bytes()),
            4 => patched[field..field + 4].copy_from_slice(&value.to_le_bytes()),
            other => panic!("unexpected TIFF dimension field type {other}"),
        }
    }
    patched
}

fn with_webp_dimensions(bytes: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut patched = bytes.to_vec();
    let packed = u32::from_le_bytes(bytes[21..25].try_into().unwrap());
    let fields = (width - 1) | ((height - 1) << 14);
    let next = (packed & 0xF000_0000) | fields;
    patched[21..25].copy_from_slice(&next.to_le_bytes());
    patched
}

fn with_dimensions(codec: Codec, bytes: &[u8], width: u32, height: u32) -> Vec<u8> {
    match codec {
        Codec::Jpeg => with_jpeg_dimensions(bytes, width, height),
        Codec::Png => with_png_dimensions(bytes, width, height),
        Codec::WebP => with_webp_dimensions(bytes, width, height),
        Codec::Bmp => with_bmp_dimensions(bytes, width, height),
        Codec::Tiff => with_tiff_dimensions(bytes, width, height),
        Codec::Gif => with_gif_dimensions(bytes, width, height),
    }
}

fn stored(codec: Codec) -> Vec<u8> {
    let pixels = block_pixels(Layout::Rgb, PARTIAL_ALPHAS);
    encode(codec, Layout::Rgb, WIDTH, HEIGHT, &pixels, None).unwrap()
}

#[test]
fn rgb_pixels_decode_to_opaque_rgba_for_every_common_codec() {
    for codec in COMMON {
        let pixels = block_pixels(Layout::Rgb, PARTIAL_ALPHAS);
        let bytes = encode(codec, Layout::Rgb, WIDTH, HEIGHT, &pixels, None).unwrap();
        let decoded = decode_default(&bytes);
        assert_dimensions(&decoded, WIDTH, HEIGHT);
        assert_rgba_within(
            decoded.pixels(),
            &as_rgba(Layout::Rgb, &pixels),
            tolerance(codec),
            &format!("{codec:?} RGB"),
        );
    }
}

#[test]
fn grayscale_pixels_decode_to_equal_rgb_channels_where_the_encoder_keeps_grey() {
    for codec in COMMON {
        let pixels = block_pixels(Layout::Grey, PARTIAL_ALPHAS);
        let bytes = encode(codec, Layout::Grey, WIDTH, HEIGHT, &pixels, None).unwrap();
        let decoded = decode_default(&bytes);
        assert_dimensions(&decoded, WIDTH, HEIGHT);
        assert_rgba_within(
            decoded.pixels(),
            &as_rgba(Layout::Grey, &pixels),
            tolerance(codec),
            &format!("{codec:?} grayscale"),
        );
    }
}

#[test]
fn alpha_survives_for_every_codec_that_carries_it() {
    for codec in COMMON {
        let alphas = if codec == Codec::Gif {
            BINARY_ALPHAS
        } else {
            PARTIAL_ALPHAS
        };
        let pixels = block_pixels(Layout::Rgba, alphas);
        let outcome = encode(codec, Layout::Rgba, WIDTH, HEIGHT, &pixels, None);
        if codec == Codec::Jpeg {
            assert!(
                outcome.is_err(),
                "JPEG must not accept an alpha colour type"
            );
            continue;
        }
        let decoded = decode_default(&outcome.unwrap());
        assert_dimensions(&decoded, WIDTH, HEIGHT);
        assert_alpha_and_visible_colour(&decoded, &pixels, codec);
    }
}

#[test]
fn exif_orientation_one_to_eight_moves_pixels_for_codecs_that_store_it() {
    for codec in [Codec::Jpeg, Codec::Png, Codec::WebP, Codec::Tiff] {
        let pixels = block_pixels(Layout::Rgb, PARTIAL_ALPHAS);
        let rgba = as_rgba(Layout::Rgb, &pixels);
        for orientation in 1..=8_u8 {
            let bytes = encode(
                codec,
                Layout::Rgb,
                WIDTH,
                HEIGHT,
                &pixels,
                Some(orientation),
            )
            .unwrap();
            let decoded = decode_default(&bytes);
            let (width, height, expected) = orient(&rgba, WIDTH, HEIGHT, orientation, 4);
            assert_dimensions(&decoded, width, height);
            assert_rgba_within(
                decoded.pixels(),
                &expected,
                tolerance(codec),
                &format!("{codec:?} orientation {orientation}"),
            );
        }
    }
}

#[test]
fn bmp_and_gif_have_no_exif_orientation_and_keep_stored_dimensions() {
    for codec in [Codec::Bmp, Codec::Gif] {
        let decoded = decode_default(&stored(codec));
        assert_dimensions(&decoded, WIDTH, HEIGHT);
    }
}

#[test]
fn tiff_multi_page_source_decodes_the_first_page_only() {
    let first = block_pixels(Layout::Rgb, PARTIAL_ALPHAS);
    let second: Vec<u8> = first.iter().map(|byte| 255 - byte).collect();
    let mut output = Cursor::new(Vec::new());
    {
        let mut encoder = TiffEncoder::new(&mut output).unwrap();
        write_tiff_page::<_, colortype::RGB8>(&mut encoder, WIDTH, HEIGHT, &first, None).unwrap();
        write_tiff_page::<_, colortype::RGB8>(&mut encoder, WIDTH, HEIGHT, &second, None).unwrap();
    }
    let bytes = output.into_inner();
    let pages = TiffDecoder::new(Cursor::new(bytes.as_slice())).unwrap();
    assert!(
        pages.more_images(),
        "the fixture must contain a second page"
    );
    let decoded = decode_default(&bytes);
    assert_dimensions(&decoded, WIDTH, HEIGHT);
    assert_rgba_within(
        decoded.pixels(),
        &as_rgba(Layout::Rgb, &first),
        0,
        "TIFF first page",
    );
}

#[test]
fn gif_decodes_the_first_frame_while_a_second_frame_is_present() {
    let first = vec![255, 0, 0, 255];
    let second = vec![0, 0, 255, 255];
    let bytes = encode_gif(vec![(1, 1, first.clone()), (1, 1, second.clone())]).unwrap();
    let frames = GifDecoder::new(Cursor::new(bytes.as_slice()))
        .unwrap()
        .into_frames()
        .collect_frames()
        .unwrap();
    assert_eq!(frames.len(), 2, "the fixture must contain a second frame");
    assert_eq!(frames[1].buffer().as_raw(), &second);
    let decoded = decode_default(&bytes);
    assert_dimensions(&decoded, 1, 1);
    assert_eq!(decoded.pixels(), first.as_slice());
}

#[test]
fn sixteen_bit_png_normalises_to_eight_bit_by_the_asserted_value() {
    let image = DynamicImage::ImageRgb16(ImageBuffer::from_pixel(
        2,
        1,
        Rgb([0xABCD_u16, 0x1234, 0xFFFF]),
    ));
    let mut output = Cursor::new(Vec::new());
    image
        .write_to(&mut output, image::ImageFormat::Png)
        .unwrap();
    let decoded = decode_default(output.get_ref());
    assert_dimensions(&decoded, 2, 1);
    assert_eq!(&decoded.pixels()[..4], &[0xAB, 0x12, 0xFF, 255]);
}

#[test]
fn sixteen_bit_tiff_normalises_to_eight_bit_by_the_asserted_value() {
    let pixels = [0xABCD_u16, 0x1234, 0xFFFF, 0xABCD, 0x1234, 0xFFFF];
    let mut output = Cursor::new(Vec::new());
    {
        let mut encoder = TiffEncoder::new(&mut output).unwrap();
        encoder
            .write_image::<colortype::RGB16>(2, 1, &pixels)
            .unwrap();
    }
    let decoded = decode_default(output.get_ref());
    assert_dimensions(&decoded, 2, 1);
    assert_eq!(&decoded.pixels()[..4], &[0xAB, 0x12, 0xFF, 255]);
}

#[test]
fn pixel_limit_accepts_exactly_the_stored_pixel_count_and_rejects_one_fewer() {
    let pixel_count = u64::from(WIDTH) * u64::from(HEIGHT);
    for codec in COMMON {
        let bytes = stored(codec);
        let at_limit = DecodeLimits::new(
            ResourceLimits::new(pixel_count, 256 * 1024 * 1024).unwrap(),
            256 * 1024 * 1024,
            1024 * 1024 * 1024,
        )
        .unwrap();
        assert_dimensions(&decode(&bytes, at_limit).unwrap(), WIDTH, HEIGHT);
        let below = DecodeLimits::new(
            ResourceLimits::new(pixel_count - 1, 256 * 1024 * 1024).unwrap(),
            256 * 1024 * 1024,
            1024 * 1024 * 1024,
        )
        .unwrap();
        assert!(
            matches!(
                decode(&bytes, below),
                Err(DecodeError::Contract(ContractError::ResourceLimitExceeded))
            ),
            "{codec:?} must reject one pixel above its limit"
        );
    }
}

#[test]
fn encoded_byte_limit_accepts_exact_length_and_rejects_one_byte_less() {
    for codec in COMMON {
        let bytes = stored(codec);
        let length = u64::try_from(bytes.len()).unwrap();
        let exact =
            DecodeLimits::new(ResourceLimits::default(), length, 1024 * 1024 * 1024).unwrap();
        assert_dimensions(&decode(&bytes, exact).unwrap(), WIDTH, HEIGHT);
        let short =
            DecodeLimits::new(ResourceLimits::default(), length - 1, 1024 * 1024 * 1024).unwrap();
        assert!(
            matches!(
                decode(&bytes, short),
                Err(DecodeError::EncodedInputTooLarge)
            ),
            "{codec:?} must reject input one byte above the encoded limit"
        );
    }
}

#[test]
fn fifty_megapixel_header_passes_the_limit_at_the_boundary_and_is_rejected_one_above() {
    for codec in COMMON {
        let base = stored(codec);
        let at = with_dimensions(codec, &base, 10_000, 5_000);
        assert_limit_passed_then_truncated(codec, &at, DecodeLimits::default(), 10_000, 5_000);
        let above = with_dimensions(codec, &base, 10_000, 5_001);
        assert!(
            matches!(
                decode(&above, DecodeLimits::default()),
                Err(DecodeError::Contract(ContractError::ResourceLimitExceeded))
            ),
            "{codec:?} above 50 MP must be rejected from the header"
        );
    }
}

#[test]
fn fifty_megapixel_png_decodes_fully_at_the_boundary() {
    let (width, height) = (10_000_u32, 5_000_u32);
    let grey = vec![128_u8; usize::try_from(u64::from(width) * u64::from(height)).unwrap()];
    let bytes = encode(Codec::Png, Layout::Grey, width, height, &grey, None).unwrap();
    let decoded = decode_default(&bytes);
    assert_dimensions(&decoded, width, height);
    assert!(
        decoded
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel == [128, 128, 128, 255]),
        "50 MP PNG pixels must decode to the encoded grey level"
    );
}

#[test]
fn per_axis_decoder_limit_accepts_non_square_images_in_both_orientations() {
    for (width, height) in [(WIDTH, HEIGHT), (HEIGHT, WIDTH)] {
        let pixels = vec![90_u8; usize::try_from(width * height * 3).unwrap()];
        for codec in COMMON {
            let bytes = encode(codec, Layout::Rgb, width, height, &pixels, None).unwrap();
            assert_dimensions(&decode_default(&bytes), width, height);
        }
    }
}

#[test]
fn lowered_sixteen_megapixel_browser_limit_rejects_a_crafted_header_above_it() {
    let browser = DecodeLimits::new(
        ResourceLimits::new(16_000_000, 256 * 1024 * 1024).unwrap(),
        256 * 1024 * 1024,
        1024 * 1024 * 1024,
    )
    .unwrap();
    for codec in COMMON {
        let base = stored(codec);
        let at = with_dimensions(codec, &base, 4_000, 4_000);
        assert_limit_passed_then_truncated(codec, &at, browser, 4_000, 4_000);
        let above = with_dimensions(codec, &base, 4_000, 4_001);
        assert!(
            matches!(
                decode(&above, browser),
                Err(DecodeError::Contract(ContractError::ResourceLimitExceeded))
            ),
            "{codec:?} above 16 MP must be rejected under the browser limit"
        );
    }
}

/// Orientation 6 needs a native-sized working copy, which the decoded byte budget must count.
///
/// The budget is a byte total, so it does not detect a width/height swap of the output.
/// Output dimensions are checked by `exif_orientation_one_to_eight_moves_pixels_for_codecs_that_store_it`.
#[test]
fn orientation_six_working_copy_counts_toward_the_decoded_byte_budget() {
    let rgb = block_pixels(Layout::Rgb, PARTIAL_ALPHAS);
    let bytes = encode(Codec::Png, Layout::Rgb, WIDTH, HEIGHT, &rgb, Some(6)).unwrap();
    let untransformed_working_bytes = 3_072;
    let tight = DecodeLimits::new(
        ResourceLimits::default(),
        256 * 1024 * 1024,
        untransformed_working_bytes,
    )
    .unwrap();
    assert!(
        matches!(
            decode(&bytes, tight),
            Err(DecodeError::Contract(ContractError::ResourceLimitExceeded))
        ),
        "a 90-degree swap must add its working copy to the decoded budget"
    );
    let transformed_working_bytes = 4_608;
    let fitting = DecodeLimits::new(
        ResourceLimits::default(),
        256 * 1024 * 1024,
        transformed_working_bytes,
    )
    .unwrap();
    let decoded = decode(&bytes, fitting).unwrap();
    assert_dimensions(&decoded, HEIGHT, WIDTH);
}

/// Checks that the exact product pixel count passes after an orientation-6 transform.
///
/// A product pixel limit is symmetric, so the limit itself cannot detect a width/height
/// swap. The decoder's per-axis limit is applied to the raw 24x16 input, not to the output.
#[test]
fn orientation_six_output_passes_an_exact_product_pixel_limit() {
    let rgb = block_pixels(Layout::Rgb, PARTIAL_ALPHAS);
    let bytes = encode(Codec::Png, Layout::Rgb, WIDTH, HEIGHT, &rgb, Some(6)).unwrap();
    let pixel_limit =
        ResourceLimits::new(u64::from(WIDTH) * u64::from(HEIGHT), 256 * 1024 * 1024).unwrap();
    let swapped = DecodeLimits::new(pixel_limit, 256 * 1024 * 1024, 1024 * 1024 * 1024).unwrap();
    assert_dimensions(&decode(&bytes, swapped).unwrap(), HEIGHT, WIDTH);
}

const ANIMATION_FRAMES: usize = 2;

fn animation_frames() -> (Vec<u8>, Vec<u8>) {
    let first = as_rgba(Layout::Rgb, &block_pixels(Layout::Rgb, PARTIAL_ALPHAS));
    let second = first
        .iter()
        .enumerate()
        .map(|(index, &byte)| if index % 4 == 3 { 255 } else { 255 - byte })
        .collect();
    (first, second)
}

fn encode_apng(first: &[u8], second: &[u8]) -> TestResult<Vec<u8>> {
    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, WIDTH, HEIGHT);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_animated(2, 0)?;
    let mut writer = encoder.write_header()?;
    writer.write_image_data(first)?;
    writer.write_image_data(second)?;
    writer.finish()?;
    Ok(output)
}

fn lossless_frame(rgba: &[u8]) -> TestResult<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    WebPEncoder::new_lossless(&mut output).write_image(
        rgba,
        WIDTH,
        HEIGHT,
        ExtendedColorType::Rgba8,
    )?;
    let bytes = output.into_inner();
    let mut offset = 12;
    while offset + 8 <= bytes.len() {
        let size = usize::try_from(u32::from_le_bytes(
            bytes[offset + 4..offset + 8].try_into()?,
        ))?;
        let padded = size + (size & 1);
        if &bytes[offset..offset + 4] == b"VP8L" {
            return Ok(bytes[offset..offset + 8 + padded].to_vec());
        }
        offset += 8 + padded;
    }
    Err("the lossless WebP encoder wrote no VP8L chunk".into())
}

fn riff_chunk(fourcc: [u8; 4], payload: &[u8]) -> TestResult<Vec<u8>> {
    let mut chunk = fourcc.to_vec();
    chunk.extend_from_slice(&u32::try_from(payload.len())?.to_le_bytes());
    chunk.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        chunk.push(0);
    }
    Ok(chunk)
}

fn u24(value: u32) -> [u8; 3] {
    let bytes = value.to_le_bytes();
    [bytes[0], bytes[1], bytes[2]]
}

/// Wraps lossless VP8L frames in the extended WebP container (VP8X, ANIM, one ANMF per frame).
fn animated_webp(frames: &[Vec<u8>], blend: u8) -> TestResult<Vec<u8>> {
    let mut canvas = vec![0x12, 0, 0, 0];
    canvas.extend_from_slice(&u24(WIDTH - 1));
    canvas.extend_from_slice(&u24(HEIGHT - 1));
    let mut body = b"WEBP".to_vec();
    body.extend(riff_chunk(*b"VP8X", &canvas)?);
    body.extend(riff_chunk(*b"ANIM", &[0; 6])?);
    for frame in frames {
        let mut anmf = Vec::new();
        anmf.extend_from_slice(&u24(0));
        anmf.extend_from_slice(&u24(0));
        anmf.extend_from_slice(&u24(WIDTH - 1));
        anmf.extend_from_slice(&u24(HEIGHT - 1));
        anmf.extend_from_slice(&u24(100));
        anmf.push(blend);
        anmf.extend_from_slice(frame);
        body.extend(riff_chunk(*b"ANMF", &anmf)?);
    }
    let mut output = b"RIFF".to_vec();
    output.extend_from_slice(&u32::try_from(body.len())?.to_le_bytes());
    output.extend(body);
    Ok(output)
}

#[test]
fn animated_png_decodes_its_first_frame_and_the_second_frame_is_a_sensitive_control() {
    let (first, second) = animation_frames();
    let bytes = encode_apng(&first, &second).unwrap();
    let frames = image::codecs::png::PngDecoder::new(Cursor::new(bytes.as_slice()))
        .unwrap()
        .apng()
        .unwrap()
        .into_frames()
        .collect_frames()
        .unwrap();
    assert_eq!(
        frames.len(),
        ANIMATION_FRAMES,
        "the fixture must contain two frames"
    );
    assert_eq!(frames[1].buffer().as_raw(), &second);
    let decoded = decode_default(&bytes);
    assert_dimensions(&decoded, WIDTH, HEIGHT);
    assert_eq!(
        decoded.pixels(),
        first.as_slice(),
        "APNG must decode to its first frame"
    );
    assert_ne!(
        decoded.pixels(),
        second.as_slice(),
        "the second frame must differ from the first"
    );
}

/// The ANMF blend bit set (do not blend): a full-canvas opaque frame is stored as-is.
const NO_BLEND: u8 = 0x02;

#[test]
fn animated_webp_without_blending_decodes_its_first_frame_exactly() {
    let (first, second) = animation_frames();
    let bytes = animated_webp(
        &[
            lossless_frame(&first).unwrap(),
            lossless_frame(&second).unwrap(),
        ],
        NO_BLEND,
    )
    .unwrap();
    let frames = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes.as_slice()))
        .unwrap()
        .into_frames()
        .collect_frames()
        .unwrap();
    assert_eq!(
        frames.len(),
        ANIMATION_FRAMES,
        "the fixture must contain two frames"
    );
    assert_eq!(frames[1].buffer().as_raw(), &second);
    let decoded = decode_default(&bytes);
    assert_dimensions(&decoded, WIDTH, HEIGHT);
    assert_eq!(
        decoded.pixels(),
        first.as_slice(),
        "animated WebP must decode to its first frame"
    );
    assert_ne!(
        decoded.pixels(),
        second.as_slice(),
        "the second frame must differ from the first"
    );
}

/// Known deviation, pinned: with the alpha-blend flag (the default), the decoder returns the
/// first frame one level low on each opaque channel. Recorded as a finding in format-coverage.md.
#[test]
fn animated_webp_with_alpha_blending_is_within_one_level_of_its_first_frame() {
    let (first, second) = animation_frames();
    let bytes = animated_webp(
        &[
            lossless_frame(&first).unwrap(),
            lossless_frame(&second).unwrap(),
        ],
        0,
    )
    .unwrap();
    let decoded = decode_default(&bytes);
    assert_dimensions(&decoded, WIDTH, HEIGHT);
    assert_rgba_within(
        decoded.pixels(),
        &first,
        1,
        "animated WebP alpha-blended first frame",
    );
    assert_ne!(
        decoded.pixels(),
        first.as_slice(),
        "the deviation is pinned until the decoder changes"
    );
    assert_ne!(
        decoded.pixels(),
        second.as_slice(),
        "the second frame must differ from the first"
    );
}

#[cfg(feature = "heif")]
mod heif_native {
    use super::*;

    use libheif_rs::HeifContext;

    const HEIC: &[u8] = include_bytes!("fixtures/heif/sample.heic");
    const AVIF: &[u8] = include_bytes!("fixtures/heif/sample.avif");
    const HEIC_ALPHA: &[u8] = include_bytes!("fixtures/heif/sample_alpha.heic");
    const AVIF_ALPHA: &[u8] = include_bytes!("fixtures/heif/sample_alpha.avif");
    const AVIF_10BIT: &[u8] = include_bytes!("fixtures/heif/sample_10bit.avif");
    const HEIC_IROT90: &[u8] = include_bytes!("fixtures/heif/sample_irot90.heic");
    const HEIC_IMIR_AXIS0: &[u8] = include_bytes!("fixtures/heif/sample_imir_axis0.heic");
    const HEIC_IMIR_AXIS1: &[u8] = include_bytes!("fixtures/heif/sample_imir_axis1.heic");
    const AVIF_IROT270: &[u8] = include_bytes!("fixtures/heif/sample_irot270.avif");

    const SAMPLE_WIDTH: u32 = 64;
    const SAMPLE_HEIGHT: u32 = 48;
    const NATIVE_TOLERANCE: u8 = 12;
    const TRANSFORM_TOLERANCE: u8 = 4;

    fn with_ispe(bytes: &[u8], width: u32, height: u32) -> Vec<u8> {
        let mut patched = bytes.to_vec();
        let position = patched
            .windows(4)
            .position(|window| window == b"ispe")
            .unwrap();
        patched[position + 8..position + 12].copy_from_slice(&width.to_be_bytes());
        patched[position + 12..position + 16].copy_from_slice(&height.to_be_bytes());
        patched
    }

    fn luma_bits(bytes: &[u8]) -> u8 {
        HeifContext::read_from_bytes(bytes)
            .unwrap()
            .primary_image_handle()
            .unwrap()
            .luma_bits_per_pixel()
    }

    fn corner(decoded: &DecodedSource, x: u32, y: u32) -> [u8; 4] {
        let width = usize::try_from(decoded.metadata().dimensions().width()).unwrap();
        let start = (usize::try_from(y).unwrap() * width + usize::try_from(x).unwrap()) * 4;
        decoded.pixels()[start..start + 4].try_into().unwrap()
    }

    fn assert_corner_near(decoded: &DecodedSource, x: u32, y: u32, expected: [u8; 4], label: &str) {
        let actual = corner(decoded, x, y);
        assert_rgba_within(&actual, &expected, NATIVE_TOLERANCE, label);
    }

    #[test]
    fn plain_heic_and_avif_decode_to_the_gradient_corners() {
        let heic = decode_default(HEIC);
        assert_dimensions(&heic, SAMPLE_WIDTH, SAMPLE_HEIGHT);
        assert_corner_near(&heic, 0, 0, [0, 2, 121, 255], "HEIC top-left");
        assert_corner_near(&heic, 63, 47, [249, 254, 127, 255], "HEIC bottom-right");
        let avif = decode_default(AVIF);
        assert_dimensions(&avif, SAMPLE_WIDTH, SAMPLE_HEIGHT);
        assert_corner_near(&avif, 0, 0, [1, 0, 129, 255], "AVIF top-left");
        assert_corner_near(&avif, 63, 47, [255, 254, 130, 255], "AVIF bottom-right");
    }

    #[test]
    fn heic_and_avif_alpha_channels_survive_decoding() {
        let heic = decode_default(HEIC_ALPHA);
        assert_dimensions(&heic, SAMPLE_WIDTH, SAMPLE_HEIGHT);
        assert_corner_near(&heic, 0, 0, [0, 2, 121, 255], "HEIC alpha opaque corner");
        assert_corner_near(
            &heic,
            63,
            0,
            [249, 5, 129, 128],
            "HEIC alpha falling corner",
        );
        let avif = decode_default(AVIF_ALPHA);
        assert_dimensions(&avif, SAMPLE_WIDTH, SAMPLE_HEIGHT);
        assert_corner_near(&avif, 0, 0, [1, 0, 129, 255], "AVIF alpha opaque corner");
        assert_corner_near(
            &avif,
            63,
            0,
            [249, 3, 126, 127],
            "AVIF alpha falling corner",
        );
    }

    #[test]
    fn ten_bit_avif_source_normalises_to_eight_bit_corners() {
        assert_eq!(luma_bits(AVIF_10BIT), 10);
        let decoded = decode_default(AVIF_10BIT);
        assert_dimensions(&decoded, SAMPLE_WIDTH, SAMPLE_HEIGHT);
        assert_corner_near(&decoded, 0, 0, [1, 1, 126, 255], "10-bit AVIF top-left");
        assert_corner_near(
            &decoded,
            63,
            47,
            [255, 253, 131, 255],
            "10-bit AVIF bottom-right",
        );
    }

    fn assert_transform_matches_plain(
        plain: &DecodedSource,
        transformed: &[u8],
        orientation: u8,
        label: &str,
    ) {
        let decoded = decode_default(transformed);
        let (width, height, expected) =
            orient(plain.pixels(), SAMPLE_WIDTH, SAMPLE_HEIGHT, orientation, 4);
        assert_dimensions(&decoded, width, height);
        assert_rgba_within(decoded.pixels(), &expected, TRANSFORM_TOLERANCE, label);
    }

    #[test]
    fn heic_irot_ninety_degrees_anticlockwise_rotates_the_gradient() {
        let plain = decode_default(HEIC);
        assert_transform_matches_plain(&plain, HEIC_IROT90, 8, "HEIC irot 90");
        let rotated = decode_default(HEIC_IROT90);
        assert_corner_near(&rotated, 0, 0, [249, 5, 129, 255], "HEIC irot 90 top-left");
    }

    #[test]
    fn heic_imir_mirrors_top_to_bottom_on_axis_zero_and_left_to_right_on_axis_one() {
        let plain = decode_default(HEIC);
        assert_transform_matches_plain(&plain, HEIC_IMIR_AXIS0, 4, "HEIC imir axis 0");
        assert_transform_matches_plain(&plain, HEIC_IMIR_AXIS1, 2, "HEIC imir axis 1");
        let top_bottom = decode_default(HEIC_IMIR_AXIS0);
        assert_corner_near(
            &top_bottom,
            0,
            0,
            [4, 251, 127, 255],
            "HEIC imir axis 0 corner",
        );
        let left_right = decode_default(HEIC_IMIR_AXIS1);
        assert_corner_near(
            &left_right,
            0,
            0,
            [249, 5, 129, 255],
            "HEIC imir axis 1 corner",
        );
    }

    #[test]
    fn avif_irot_two_seventy_degrees_anticlockwise_rotates_the_gradient() {
        let plain = decode_default(AVIF);
        assert_transform_matches_plain(&plain, AVIF_IROT270, 6, "AVIF irot 270");
    }

    /// The header passes the limit, and the truncated payload then fails in libheif.
    fn assert_heif_limit_passed(bytes: &[u8], limits: DecodeLimits, label: &str) {
        assert!(
            matches!(
                decode(bytes, limits),
                Err(DecodeError::Image(ImageError::Decoding(_)))
            ),
            "{label}: expected libheif's size-mismatch error after the limit check"
        );
    }

    #[test]
    fn fifty_megapixel_ispe_declaration_is_rejected_before_pixel_decode() {
        let heic = with_ispe(HEIC, 10_000, 5_001);
        assert!(matches!(
            decode(&heic, DecodeLimits::default()),
            Err(DecodeError::Contract(ContractError::ResourceLimitExceeded))
        ));
        let avif = with_ispe(AVIF, 10_000, 5_001);
        assert!(matches!(
            decode(&avif, DecodeLimits::default()),
            Err(DecodeError::Contract(ContractError::ResourceLimitExceeded))
        ));
    }

    #[test]
    fn fifty_megapixel_ispe_boundary_passes_the_limit() {
        for (bytes, label) in [(HEIC, "HEIC"), (AVIF, "AVIF")] {
            let at = with_ispe(bytes, 10_000, 5_000);
            assert_heif_limit_passed(&at, DecodeLimits::default(), label);
        }
    }

    #[test]
    fn lowered_sixteen_megapixel_limit_accepts_the_sample_and_rejects_a_larger_declaration() {
        let browser = DecodeLimits::new(
            ResourceLimits::new(16_000_000, 256 * 1024 * 1024).unwrap(),
            256 * 1024 * 1024,
            1024 * 1024 * 1024,
        )
        .unwrap();
        assert_dimensions(&decode(HEIC, browser).unwrap(), SAMPLE_WIDTH, SAMPLE_HEIGHT);
        for bytes in [HEIC, AVIF] {
            let above = with_ispe(bytes, 4_000, 4_001);
            assert!(matches!(
                decode(&above, browser),
                Err(DecodeError::Contract(ContractError::ResourceLimitExceeded))
            ));
        }
    }

    #[test]
    fn exact_pixel_limit_accepts_the_sample_and_one_fewer_rejects() {
        let pixel_count = u64::from(SAMPLE_WIDTH) * u64::from(SAMPLE_HEIGHT);
        for bytes in [HEIC, AVIF] {
            let at = DecodeLimits::new(
                ResourceLimits::new(pixel_count, 256 * 1024 * 1024).unwrap(),
                256 * 1024 * 1024,
                1024 * 1024 * 1024,
            )
            .unwrap();
            assert_dimensions(&decode(bytes, at).unwrap(), SAMPLE_WIDTH, SAMPLE_HEIGHT);
            let below = DecodeLimits::new(
                ResourceLimits::new(pixel_count - 1, 256 * 1024 * 1024).unwrap(),
                256 * 1024 * 1024,
                1024 * 1024 * 1024,
            )
            .unwrap();
            assert!(matches!(
                decode(bytes, below),
                Err(DecodeError::Contract(ContractError::ResourceLimitExceeded))
            ));
        }
    }

    #[test]
    fn encoded_byte_limit_accepts_exact_length_and_rejects_one_byte_less_for_heif() {
        for bytes in [HEIC, AVIF] {
            let length = u64::try_from(bytes.len()).unwrap();
            let exact =
                DecodeLimits::new(ResourceLimits::default(), length, 1024 * 1024 * 1024).unwrap();
            assert_dimensions(&decode(bytes, exact).unwrap(), SAMPLE_WIDTH, SAMPLE_HEIGHT);
            let short =
                DecodeLimits::new(ResourceLimits::default(), length - 1, 1024 * 1024 * 1024)
                    .unwrap();
            assert!(matches!(
                decode(bytes, short),
                Err(DecodeError::EncodedInputTooLarge)
            ));
        }
    }

    const GREY_HEIC: &[u8] = include_bytes!("fixtures/heif/sample_grey.heic");
    const GREY_AVIF: &[u8] = include_bytes!("fixtures/heif/sample_grey.avif");
    const TWO_HEIC: &[u8] = include_bytes!("fixtures/heif/sample_two_images.heic");
    const TWO_AVIF: &[u8] = include_bytes!("fixtures/heif/sample_two_images.avif");

    fn with_primary_item(bytes: &[u8], item_id: u16) -> Vec<u8> {
        let mut patched = bytes.to_vec();
        let position = patched
            .windows(4)
            .position(|window| window == b"pitm")
            .unwrap();
        patched[position + 8..position + 10].copy_from_slice(&item_id.to_be_bytes());
        patched
    }

    #[test]
    fn grey_heic_and_avif_fixtures_decode_to_equal_rgb_channels() {
        for (bytes, label) in [(GREY_HEIC, "HEIC grey"), (GREY_AVIF, "AVIF grey")] {
            let decoded = decode_default(bytes);
            assert_dimensions(&decoded, SAMPLE_WIDTH, SAMPLE_HEIGHT);
            for pixel in decoded.pixels().chunks_exact(4) {
                let spread = pixel[0].abs_diff(pixel[1]).max(pixel[1].abs_diff(pixel[2]));
                assert!(
                    spread <= 6,
                    "{label}: channels differ by {spread} in {pixel:?}"
                );
            }
            assert_corner_near(&decoded, 0, 0, [0, 0, 0, 255], label);
            assert_corner_near(&decoded, 63, 0, [127, 127, 127, 255], label);
            assert_corner_near(&decoded, 0, 47, [127, 127, 127, 255], label);
            assert_corner_near(&decoded, 63, 47, [255, 255, 255, 255], label);
        }
    }

    #[test]
    fn grey_fixtures_keep_chroma_planes_so_they_are_not_monochrome() {
        for bytes in [GREY_HEIC, GREY_AVIF] {
            let handle = HeifContext::read_from_bytes(bytes)
                .unwrap()
                .primary_image_handle()
                .unwrap();
            assert_eq!(handle.chroma_bits_per_pixel(), 8);
        }
    }

    #[test]
    fn heif_decodes_the_primary_item_even_when_it_is_not_the_first_item() {
        for (two_images, plain, label) in [(TWO_HEIC, HEIC, "HEIC"), (TWO_AVIF, AVIF, "AVIF")] {
            let primary = decode_default(two_images);
            assert_dimensions(&primary, SAMPLE_WIDTH, SAMPLE_HEIGHT);
            assert_corner_near(&primary, 0, 0, [255, 255, 128, 255], label);
            assert_corner_near(&primary, 63, 0, [0, 255, 128, 255], label);
            assert_corner_near(&primary, 0, 47, [255, 0, 128, 255], label);
            assert_corner_near(&primary, 63, 47, [0, 0, 128, 255], label);
            let first = decode_default(&with_primary_item(two_images, 1));
            assert_eq!(
                first.pixels(),
                decode_default(plain).pixels(),
                "{label} item 1"
            );
            assert_ne!(first.pixels(), primary.pixels(), "{label} primary differs");
        }
    }

    /// Byte budget only: 48x64 and 64x48 outputs have the same byte count, so a swap is not observed here.
    #[test]
    fn transformed_heic_byte_budget_is_exactly_twice_the_rgba_output() {
        let output_bytes = 48 * 64 * 4;
        let at = DecodeLimits::new(
            ResourceLimits::default(),
            256 * 1024 * 1024,
            output_bytes * 2,
        )
        .unwrap();
        assert_dimensions(&decode(HEIC_IROT90, at).unwrap(), 48, 64);
        let below = DecodeLimits::new(
            ResourceLimits::default(),
            256 * 1024 * 1024,
            output_bytes * 2 - 1,
        )
        .unwrap();
        assert!(matches!(
            decode(HEIC_IROT90, below),
            Err(DecodeError::Contract(ContractError::ResourceLimitExceeded))
        ));
    }

    /// The HEIC pixel limit is product-based and reads the raw `ispe` size, not the
    /// rotated output, so this test cannot tell raw from post-transform dimensions.
    /// Rotated HEIC output dimensions are checked by `heic_irot_ninety_degrees_anticlockwise_rotates_the_gradient`.
    #[test]
    fn rotated_heic_ispe_product_is_checked_against_the_pixel_limit() {
        let raw_landscape_fits = with_ispe(HEIC_IROT90, 5_000, 10_000);
        assert_heif_limit_passed(&raw_landscape_fits, DecodeLimits::default(), "rotated HEIC");
        let raw_overflows = with_ispe(HEIC_IROT90, 10_000, 5_001);
        assert!(matches!(
            decode(&raw_overflows, DecodeLimits::default()),
            Err(DecodeError::Contract(ContractError::ResourceLimitExceeded))
        ));
    }
}
