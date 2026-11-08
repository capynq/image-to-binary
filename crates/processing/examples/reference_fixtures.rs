//! Writes native preview references and generated encoded fixtures (JSON, base64)
//! for the WASM binding tests.

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::codecs::gif::GifEncoder;
use image::{Delay, DynamicImage, Frame, ImageBuffer, ImageFormat, Luma, Rgba};
use image_to_binary_adapters::session::{
    EncodedPlan, PreviewSettings, Stage, DEFAULT_PIXEL_BUDGET,
};
use image_to_binary_adapters::{ArtworkPngTask, ArtworkTextTask, PngTaskLimits};
use image_to_binary_core::{ContractError, Dimensions, Generation, Ramp, RenderMode, Sampling};
use image_to_binary_processing::{
    decode, ColorMode, DecodeError, DecodeLimits, DecodedSource, RenderOptions, RenderStyle,
    RenderedFrame, Renderer, Rgb, SampleGrid, Sampler,
};
use tiff::encoder::colortype::RGB8;
use tiff::encoder::TiffEncoder;

struct Case {
    name: &'static str,
    width: u32,
    height: u32,
    cell_size: u8,
    font_size: u8,
    sampling: Sampling,
    thresholds: &'static [u8],
}

const CASES: [Case; 4] = [
    Case {
        name: "classic_61x47",
        width: 61,
        height: 47,
        cell_size: 7,
        font_size: 14,
        sampling: Sampling::Classic,
        thresholds: &[64, 192],
    },
    Case {
        name: "average_53x71",
        width: 53,
        height: 71,
        cell_size: 6,
        font_size: 18,
        sampling: Sampling::Average,
        thresholds: &[64, 192],
    },
    Case {
        name: "classic_wide_bounded",
        width: 3005,
        height: 10,
        cell_size: 5,
        font_size: 10,
        sampling: Sampling::Classic,
        thresholds: &[64, 192],
    },
    Case {
        name: "average_tall_bounded",
        width: 7,
        height: 2400,
        cell_size: 5,
        font_size: 12,
        sampling: Sampling::Average,
        thresholds: &[64, 192],
    },
];

fn rgba(width: u32, height: u32) -> Vec<u8> {
    let span_x = (width - 1).max(1);
    let span_y = (height - 1).max(1);
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            for channel in [
                x * 255 / span_x,
                y * 255 / span_y,
                (x * 37 + y * 91) % 256,
                255,
            ] {
                pixels.push(u8::try_from(channel).expect("channel fits in u8"));
            }
        }
    }
    pixels
}

/// Deterministic large source whose glyph output exceeds one browser chunk. The
/// JS suite regenerates the same pixels from `largePixel`.
const LARGE_SIDE: u32 = 1200;

fn large_rgba() -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..LARGE_SIDE {
        for x in 0..LARGE_SIDE {
            let red = (x * 131 + y * 137) ^ (x * y * 7);
            let green = (x * x + y * 3) ^ (y * y * 5);
            let blue = (x ^ y) * 29 + (x * 11 + y * 13) % 61;
            for channel in [red % 256, green % 256, blue % 256, 255] {
                pixels.push(u8::try_from(channel).expect("channel fits in u8"));
            }
        }
    }
    pixels
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = u32::from(chunk[0]);
        let second = u32::from(chunk.get(1).copied().unwrap_or(0));
        let third = u32::from(chunk.get(2).copied().unwrap_or(0));
        let value = (first << 16) | (second << 8) | third;
        for index in 0_usize..4 {
            if index <= chunk.len() {
                let sextet = u8::try_from((value >> (18 - 6 * index)) & 63).expect("six bits");
                output.push(char::from(ALPHABET[usize::from(sextet)]));
            } else {
                output.push('=');
            }
        }
    }
    output
}

fn verify_nearest_center(
    export: &RenderedFrame,
    preview: &RenderedFrame,
) -> Result<(), Box<dyn Error>> {
    let export_width = u64::from(export.width());
    let export_height = u64::from(export.height());
    let preview_width = u64::from(preview.width());
    let preview_height = u64::from(preview.height());
    for y in 0..preview_height {
        let source_y = ((y * 2 + 1) * export_height / (preview_height * 2)).min(export_height - 1);
        for x in 0..preview_width {
            let source_x = ((x * 2 + 1) * export_width / (preview_width * 2)).min(export_width - 1);
            let from = usize::try_from((source_y * export_width + source_x) * 4)?;
            let to = usize::try_from((y * preview_width + x) * 4)?;
            if export.pixels()[from..from + 4] != preview.pixels()[to..to + 4] {
                return Err(format!("preview pixel ({x}, {y}) differs from the export").into());
            }
        }
    }
    Ok(())
}

const EXPORT_GENERATION: Generation = Generation::new(0);
const EXPORT_LIMITS: PngTaskLimits =
    PngTaskLimits::new(256 * 1024 * 1024, 1024 * 1024, 256 * 1024 * 1024);

/// Encodes the grid through the native `ArtworkPngTask` path in one advance.
fn native_export(
    grid: &Arc<SampleGrid>,
    options: RenderOptions,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut task = ArtworkPngTask::new(
        Arc::clone(grid),
        Renderer::new()?,
        options,
        EXPORT_GENERATION,
        EXPORT_LIMITS,
    )?;
    task.advance(EXPORT_GENERATION, u64::MAX)?;
    let mut bytes = Vec::new();
    for chunk in task.drain_chunks()? {
        bytes.extend(chunk);
    }
    for chunk in task.finish(EXPORT_GENERATION)? {
        bytes.extend(chunk);
    }
    Ok(bytes)
}

/// A threshold, its native preview frame, and its native PNG export.
type Variant = (u8, RenderedFrame, Vec<u8>);

fn render_case(case: &Case, pixels: Vec<u8>) -> Result<Vec<Variant>, Box<dyn Error>> {
    let dimensions = Dimensions::new(case.width, case.height)?;
    let source = DecodedSource::from_rgba(dimensions, pixels, DecodeLimits::default())?;
    let mut sampler = Sampler::new(Arc::new(source));
    let mut renderer = Renderer::new()?;
    let grid = match case.sampling {
        Sampling::Classic => sampler.sample(case.cell_size, Sampling::Classic)?,
        Sampling::Average => {
            let geometry = renderer.aspect_geometry(case.cell_size, case.font_size)?;
            sampler.sample_average_rect(case.cell_size, u32::from(geometry.source_cell_height))?
        }
    };
    let mut frames = Vec::new();
    for &threshold in case.thresholds {
        let options = RenderOptions {
            cell_size: case.cell_size,
            font_size: case.font_size,
            threshold,
            sampling: case.sampling,
            ..RenderOptions::default()
        };
        let preview = renderer.render_preview(&grid, options)?;
        let export = renderer.render(&grid, options)?;
        verify_nearest_center(&export, &preview)?;
        let png = native_export(&grid, options)?;
        frames.push((threshold, preview, png));
    }
    if frames.len() > 1 && frames[0].1.pixels() == frames[1].1.pixels() {
        return Err(format!("{}: threshold variants must differ", case.name).into());
    }
    Ok(frames)
}

const PREVIEW_CELL: u8 = 5;
const PREVIEW_FONT: u8 = 10;
const PREVIEW_THRESHOLD: u8 = 128;
const UI_CELL: u8 = 12;
const UI_FONT: u8 = 16;
const UI_THRESHOLD: u8 = 128;
const ENCODED_WIDTH: u32 = 23;
const ENCODED_HEIGHT: u32 = 17;
const PAGE_SIZE: u32 = 10;
const PIXEL_LIMIT_SIDE: u32 = 7072;

struct Fixture {
    name: &'static str,
    bytes: Vec<u8>,
    white_first: bool,
}

struct Rejected {
    name: &'static str,
    code: &'static str,
    bytes: Vec<u8>,
}

fn encode(image: &DynamicImage, format: ImageFormat) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut output = Cursor::new(Vec::new());
    image.write_to(&mut output, format)?;
    Ok(output.into_inner())
}

fn exif_oriented(mut jpeg: Vec<u8>) -> Result<Vec<u8>, Box<dyn Error>> {
    let exif = [
        b'E', b'x', b'i', b'f', 0, 0, b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0,
        0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
    ];
    let mut segment = vec![0xff, 0xe1, 0, u8::try_from(exif.len() + 2)?];
    segment.extend_from_slice(&exif);
    jpeg.splice(2..2, segment);
    Ok(jpeg)
}

fn two_frame_gif() -> Result<Vec<u8>, Box<dyn Error>> {
    let frame = |value: u8| {
        Frame::from_parts(
            ImageBuffer::from_pixel(PAGE_SIZE, PAGE_SIZE, Rgba([value, value, value, 255])),
            0,
            0,
            Delay::from_numer_denom_ms(1, 1),
        )
    };
    let mut output = Cursor::new(Vec::new());
    GifEncoder::new(&mut output).encode_frames([frame(255), frame(0)])?;
    Ok(output.into_inner())
}

const ANIMATION_SHADES: [u8; 3] = [255, 128, 0];
const ANIMATION_DELAYS: [u32; 3] = [5, 0, 50];
const LONG_FRAMES: usize = 1001;

fn shaded_gif(
    shades: &[u8],
    delays_centiseconds: &[u32],
    repeat: Option<image::codecs::gif::Repeat>,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut output = Cursor::new(Vec::new());
    let mut encoder = GifEncoder::new(&mut output);
    if let Some(repeat) = repeat {
        encoder.set_repeat(repeat)?;
    }
    let frames: Vec<Frame> = shades
        .iter()
        .zip(delays_centiseconds)
        .map(|(&shade, &centiseconds)| {
            Frame::from_parts(
                ImageBuffer::from_pixel(PAGE_SIZE, PAGE_SIZE, Rgba([shade, shade, shade, 255])),
                0,
                0,
                Delay::from_numer_denom_ms(centiseconds * 10, 1),
            )
        })
        .collect();
    encoder.encode_frames(frames)?;
    drop(encoder);
    Ok(output.into_inner())
}

fn animation_entry(name: &str, bytes: &[u8]) -> Result<String, Box<dyn Error>> {
    let animation = image_to_binary_processing::decode_animation(bytes, DecodeLimits::default())?
        .ok_or("the animated fixture has several frames")?;
    let dimensions = animation.metadata().dimensions();
    let loop_count = match animation.loop_count() {
        image_to_binary_processing::LoopCount::Infinite => "\"infinite\"".to_owned(),
        image_to_binary_processing::LoopCount::Finite(times) => times.to_string(),
    };
    let mut frames = String::new();
    for (index, frame) in animation.frames().iter().enumerate() {
        if index > 0 {
            frames.push(',');
        }
        let copy = DecodedSource::from_rgba(
            dimensions,
            frame.source().pixels().to_vec(),
            DecodeLimits::default(),
        )?;
        let (preview, _) = preview_of(copy)?;
        write!(
            frames,
            r#"{{"delay":{},"previewWidth":{},"previewHeight":{},"preview":"{}"}}"#,
            frame.delay_centiseconds(),
            preview.width(),
            preview.height(),
            base64(preview.pixels())
        )?;
    }
    Ok(format!(
        r#"{{"name":"{name}","width":{},"height":{},"bytes":"{}","loopCount":{loop_count},"frames":[{frames}]}}"#,
        dimensions.width(),
        dimensions.height(),
        base64(bytes)
    ))
}

fn long_animation_entry() -> Result<String, Box<dyn Error>> {
    let bytes = shaded_gif(&vec![255; LONG_FRAMES], &vec![1; LONG_FRAMES], None)?;
    let first = decode(&bytes, DecodeLimits::default())?;
    let dimensions = first.metadata().dimensions();
    let (preview, _) = preview_of(first)?;
    Ok(format!(
        r#"{{"name":"long_animation","width":{},"height":{},"bytes":"{}","previewWidth":{},"previewHeight":{},"preview":"{}"}}"#,
        dimensions.width(),
        dimensions.height(),
        base64(&bytes),
        preview.width(),
        preview.height(),
        base64(preview.pixels())
    ))
}

fn animation_sections() -> Result<String, Box<dyn Error>> {
    let shaded = shaded_gif(
        &ANIMATION_SHADES,
        &ANIMATION_DELAYS,
        Some(image::codecs::gif::Repeat::Infinite),
    )?;
    Ok(format!(
        "],\"animations\":[{}],\"longAnimation\":{},\"rejected\":[",
        animation_entry("shaded_gif", &shaded)?,
        long_animation_entry()?
    ))
}

fn two_page_tiff() -> Result<Vec<u8>, Box<dyn Error>> {
    let pixels = usize::try_from(PAGE_SIZE * PAGE_SIZE * 3)?;
    let mut output = Cursor::new(Vec::new());
    let mut encoder = TiffEncoder::new(&mut output)?;
    encoder.write_image::<RGB8>(PAGE_SIZE, PAGE_SIZE, &vec![255; pixels])?;
    encoder.write_image::<RGB8>(PAGE_SIZE, PAGE_SIZE, &vec![0; pixels])?;
    Ok(output.into_inner())
}

fn encoded_fixtures() -> Result<Vec<Fixture>, Box<dyn Error>> {
    let image = DynamicImage::ImageRgba8(
        ImageBuffer::from_raw(
            ENCODED_WIDTH,
            ENCODED_HEIGHT,
            rgba(ENCODED_WIDTH, ENCODED_HEIGHT),
        )
        .ok_or("fixture pixels match the dimensions")?,
    );
    let mut fixtures = Vec::new();
    for (name, format) in [
        ("bmp", ImageFormat::Bmp),
        ("gif", ImageFormat::Gif),
        ("jpeg", ImageFormat::Jpeg),
        ("png", ImageFormat::Png),
        ("tiff", ImageFormat::Tiff),
        ("webp", ImageFormat::WebP),
    ] {
        fixtures.push(Fixture {
            name,
            bytes: encode(&image, format)?,
            white_first: false,
        });
    }
    fixtures.push(Fixture {
        name: "jpeg_exif_rotated",
        bytes: exif_oriented(encode(&image, ImageFormat::Jpeg)?)?,
        white_first: false,
    });
    fixtures.push(Fixture {
        name: "gif_first_frame",
        bytes: two_frame_gif()?,
        white_first: true,
    });
    fixtures.push(Fixture {
        name: "tiff_first_page",
        bytes: two_page_tiff()?,
        white_first: true,
    });
    Ok(fixtures)
}

fn rejected_fixtures(fixtures: &[Fixture]) -> Result<Vec<Rejected>, Box<dyn Error>> {
    let png = fixtures
        .iter()
        .find(|fixture| fixture.name == "png")
        .ok_or("png fixture exists")?;
    let mut truncated = png.bytes.clone();
    truncated.truncate(40);
    if !matches!(
        decode(&truncated, DecodeLimits::default()),
        Err(DecodeError::Image(_))
    ) {
        return Err("truncated png must fail as a malformed image".into());
    }
    let oversized = DynamicImage::ImageLuma8(ImageBuffer::from_pixel(
        PIXEL_LIMIT_SIDE,
        PIXEL_LIMIT_SIDE,
        Luma([0]),
    ));
    let oversized = encode(&oversized, ImageFormat::Png)?;
    if !matches!(
        decode(&oversized, DecodeLimits::default()),
        Err(DecodeError::Contract(ContractError::ResourceLimitExceeded))
    ) {
        return Err("oversized png must exceed the pixel ceiling".into());
    }
    Ok(vec![
        Rejected {
            name: "truncated_png",
            code: "decode_failed",
            bytes: truncated,
        },
        Rejected {
            name: "pixel_limit_png",
            code: "resource_limit_exceeded",
            bytes: oversized,
        },
    ])
}

fn preview_of(source: DecodedSource) -> Result<(RenderedFrame, Vec<u8>), Box<dyn Error>> {
    let mut sampler = Sampler::new(Arc::new(source));
    let mut renderer = Renderer::new()?;
    let grid = sampler.sample(PREVIEW_CELL, Sampling::Classic)?;
    let options = RenderOptions {
        cell_size: PREVIEW_CELL,
        font_size: PREVIEW_FONT,
        threshold: PREVIEW_THRESHOLD,
        sampling: Sampling::Classic,
        ..RenderOptions::default()
    };
    let preview = renderer.render_preview(&grid, options)?;
    let export_grid = sampler.sample(UI_CELL, Sampling::Classic)?;
    let export_options = RenderOptions {
        cell_size: UI_CELL,
        font_size: UI_FONT,
        threshold: UI_THRESHOLD,
        sampling: Sampling::Classic,
        ..RenderOptions::default()
    };
    let png = native_export(&export_grid, export_options)?;
    Ok((preview, png))
}

fn encoded_entry(fixture: &Fixture) -> Result<String, Box<dyn Error>> {
    let source = decode(&fixture.bytes, DecodeLimits::default())?;
    if fixture.white_first && source.pixels()[..4] != [255, 255, 255, 255] {
        return Err(format!("{}: the first frame must be white", fixture.name).into());
    }
    let metadata = source.metadata();
    let (width, height) = (
        metadata.dimensions().width(),
        metadata.dimensions().height(),
    );
    let channels = metadata.channels();
    let normalized = base64(source.pixels());
    let (preview, png) = preview_of(source)?;
    Ok(format!(
        r#"{{"name":"{}","width":{width},"height":{height},"channels":{channels},"previewWidth":{},"previewHeight":{},"bytes":"{}","source":"{}","preview":"{}","exportSettings":{{"cellSize":{UI_CELL},"fontSize":{UI_FONT},"threshold":{UI_THRESHOLD},"sampling":"classic"}},"export":"{}"}}"#,
        fixture.name,
        preview.width(),
        preview.height(),
        base64(&fixture.bytes),
        normalized,
        base64(preview.pixels()),
        base64(&png)
    ))
}

fn large_export(sampling: Sampling) -> Result<Vec<u8>, Box<dyn Error>> {
    let case = Case {
        name: "large",
        width: LARGE_SIDE,
        height: LARGE_SIDE,
        cell_size: 5,
        font_size: 10,
        sampling,
        thresholds: &[128],
    };
    let source = DecodedSource::from_rgba(
        Dimensions::new(case.width, case.height)?,
        large_rgba(),
        DecodeLimits::default(),
    )?;
    let mut sampler = Sampler::new(Arc::new(source));
    let renderer = Renderer::new()?;
    let grid = match sampling {
        Sampling::Classic => sampler.sample(case.cell_size, Sampling::Classic)?,
        Sampling::Average => {
            let geometry = renderer.aspect_geometry(case.cell_size, case.font_size)?;
            sampler.sample_average_rect(case.cell_size, u32::from(geometry.source_cell_height))?
        }
    };
    let options = RenderOptions {
        cell_size: case.cell_size,
        font_size: case.font_size,
        threshold: 128,
        sampling,
        ..RenderOptions::default()
    };
    native_export(&grid, options)
}

/// Artwork preview of an encoded image at the studio's default settings (cell 12, font 16,
/// threshold 128, classic), rendered through the shared session the desktop and WASM use. This
/// is the pixel path a user sees, unlike the processing renderer behind the JSON `preview` field.
fn ui_preview(encoded: &[u8]) -> Result<(u32, u32, Vec<u8>), Box<dyn Error>> {
    let generation = Generation::new(1);
    let mut session =
        EncodedPlan::decode_session(encoded.to_vec()).map_err(|error| error.message())?;
    let settings =
        PreviewSettings::new(12, 16, 128, Sampling::Classic).map_err(|error| error.message())?;
    session
        .start_preview(generation, settings)
        .map_err(|error| error.message())?;
    while session
        .advance(generation, DEFAULT_PIXEL_BUDGET)
        .map_err(|error| error.message())?
        != Stage::Ready
    {}
    let mut frame = session
        .finish(generation)
        .map_err(|error| error.message())?;
    let (width, height) = (frame.width(), frame.height());
    let pixels = frame.take_pixels().map_err(|error| error.message())?;
    Ok((width, height, pixels))
}

/// One render-mode reference: a deterministic source and the settings the browser and desktop
/// send for that mode. Sampling is `average` for every non-binary mode, as the shells force it.
struct ModeCase {
    name: &'static str,
    mode: RenderMode,
    sampling: Sampling,
    ramp: Option<&'static str>,
}

const MODE_WIDTH: u32 = 64;
const MODE_HEIGHT: u32 = 40;
const MODE_CELL: u8 = 6;
const MODE_FONT: u8 = 10;
const MODE_THRESHOLD: u8 = 128;
const CUSTOM_RAMP: &str = " .oO@";

fn mode_cases() -> Result<[ModeCase; 4], Box<dyn Error>> {
    Ok([
        ModeCase {
            name: "binary_classic",
            mode: RenderMode::Binary,
            sampling: Sampling::Classic,
            ramp: None,
        },
        ModeCase {
            name: "ascii_default",
            mode: RenderMode::Ascii(Ramp::DEFAULT_ASCII),
            sampling: Sampling::Average,
            ramp: None,
        },
        ModeCase {
            name: "ascii_custom",
            mode: RenderMode::Ascii(Ramp::new(CUSTOM_RAMP)?),
            sampling: Sampling::Average,
            ramp: Some(CUSTOM_RAMP),
        },
        ModeCase {
            name: "block",
            mode: RenderMode::Block,
            sampling: Sampling::Average,
            ramp: None,
        },
    ])
}

/// Renders one mode through the processing renderer and the native PNG task directly, without
/// the shared session, so the browser and desktop output is compared with independent bytes.
fn mode_entry(case: &ModeCase) -> Result<(String, RenderedFrame, Vec<u8>), Box<dyn Error>> {
    let pixels = rgba(MODE_WIDTH, MODE_HEIGHT);
    let source = base64(&pixels);
    let dimensions = Dimensions::new(MODE_WIDTH, MODE_HEIGHT)?;
    let decoded = DecodedSource::from_rgba(dimensions, pixels, DecodeLimits::default())?;
    let mut sampler = Sampler::new(Arc::new(decoded));
    let mut renderer = Renderer::new()?;
    let grid = match case.sampling {
        Sampling::Classic => sampler.sample(MODE_CELL, Sampling::Classic)?,
        Sampling::Average => {
            let geometry = renderer.aspect_geometry(MODE_CELL, MODE_FONT)?;
            sampler.sample_average_rect(MODE_CELL, u32::from(geometry.source_cell_height))?
        }
    };
    let options = RenderOptions {
        cell_size: MODE_CELL,
        font_size: MODE_FONT,
        threshold: MODE_THRESHOLD,
        sampling: case.sampling,
        mode: case.mode,
        ..RenderOptions::default()
    };
    let preview = renderer.render_preview(&grid, options)?;
    let png = native_export(&grid, options)?;
    let sampling = match case.sampling {
        Sampling::Classic => "classic",
        Sampling::Average => "average",
    };
    let (mode, ramp) = match case.mode {
        RenderMode::Binary => ("binary", "null".to_owned()),
        RenderMode::Ascii(_) => (
            "ascii",
            format!("\"{}\"", case.ramp.unwrap_or(Ramp::DEFAULT_ASCII.as_str())),
        ),
        RenderMode::Block => ("block", "null".to_owned()),
    };
    let entry = format!(
        r#"{{"name":"{}","mode":"{mode}","ramp":{ramp},"width":{MODE_WIDTH},"height":{MODE_HEIGHT},"cellSize":{MODE_CELL},"fontSize":{MODE_FONT},"threshold":{MODE_THRESHOLD},"sampling":"{sampling}","source":"{source}","previewWidth":{},"previewHeight":{},"preview":"{}","export":"{}"}}"#,
        case.name,
        preview.width(),
        preview.height(),
        base64(preview.pixels()),
        base64(&png)
    );
    Ok((entry, preview, png))
}

/// Appends the per-mode references after `rejected`. Each mode must differ from every other,
/// otherwise the mode is not exercised.
fn mode_entries() -> Result<String, Box<dyn Error>> {
    let cases = mode_cases()?;
    let mut entries = Vec::new();
    let mut previews: Vec<RenderedFrame> = Vec::new();
    for case in &cases {
        let (entry, preview, _) = mode_entry(case)?;
        entries.push(entry);
        previews.push(preview);
    }
    for (index, preview) in previews.iter().enumerate() {
        if previews[..index]
            .iter()
            .any(|earlier| earlier.pixels() == preview.pixels())
        {
            return Err(format!(
                "{}: mode output must differ from earlier modes",
                cases[index].name
            )
            .into());
        }
    }
    Ok(entries.join(","))
}

/// One style reference: a deterministic source rendered with the classic binary path and a
/// render style, through the processing renderer and the native PNG task.
struct StyleCase {
    name: &'static str,
    style: RenderStyle,
    threshold: u8,
}

const STYLE_FOREGROUND: Rgb = Rgb {
    red: 255,
    green: 128,
    blue: 0,
};
const STYLE_BACKGROUND: Rgb = Rgb {
    red: 16,
    green: 32,
    blue: 64,
};

fn style_cases() -> [StyleCase; 5] {
    let defaults = RenderStyle::default();
    [
        StyleCase {
            name: "mono_custom_colours",
            style: RenderStyle {
                foreground: STYLE_FOREGROUND,
                background: STYLE_BACKGROUND,
                ..defaults
            },
            threshold: MODE_THRESHOLD,
        },
        StyleCase {
            name: "source_colour",
            style: RenderStyle {
                color: ColorMode::Source,
                ..defaults
            },
            threshold: MODE_THRESHOLD,
        },
        StyleCase {
            name: "contrast_raised",
            style: RenderStyle {
                contrast: 100,
                ..defaults
            },
            threshold: 96,
        },
        StyleCase {
            name: "invert",
            style: RenderStyle {
                invert: true,
                ..defaults
            },
            threshold: MODE_THRESHOLD,
        },
        StyleCase {
            name: "invert_threshold",
            style: RenderStyle {
                invert: true,
                ..defaults
            },
            threshold: 64,
        },
    ]
}

fn colour_hex(colour: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", colour.red, colour.green, colour.blue)
}

/// Renders one style case directly, without the shared session, and returns its JSON entry and
/// preview frame.
fn style_entry(case: &StyleCase) -> Result<(String, RenderedFrame, Vec<u8>), Box<dyn Error>> {
    let pixels = rgba(MODE_WIDTH, MODE_HEIGHT);
    let source = base64(&pixels);
    let dimensions = Dimensions::new(MODE_WIDTH, MODE_HEIGHT)?;
    let decoded = DecodedSource::from_rgba(dimensions, pixels, DecodeLimits::default())?;
    let mut sampler = Sampler::new(Arc::new(decoded));
    let mut renderer = Renderer::new()?;
    let grid = sampler.sample(MODE_CELL, Sampling::Classic)?;
    let options = RenderOptions {
        cell_size: MODE_CELL,
        font_size: MODE_FONT,
        threshold: case.threshold,
        sampling: Sampling::Classic,
        style: case.style,
        ..RenderOptions::default()
    };
    let preview = renderer.render_preview(&grid, options)?;
    let png = native_export(&grid, options)?;
    let color_mode = match case.style.color {
        ColorMode::Monochrome => "monochrome",
        ColorMode::Source => "source",
    };
    let entry = format!(
        r#"{{"name":"{}","foreground":"{}","background":"{}","colorMode":"{color_mode}","contrast":{},"invert":{},"threshold":{},"width":{MODE_WIDTH},"height":{MODE_HEIGHT},"cellSize":{MODE_CELL},"fontSize":{MODE_FONT},"sampling":"classic","source":"{source}","previewWidth":{},"previewHeight":{},"preview":"{}","export":"{}"}}"#,
        case.name,
        colour_hex(case.style.foreground),
        colour_hex(case.style.background),
        case.style.contrast,
        case.style.invert,
        case.threshold,
        preview.width(),
        preview.height(),
        base64(preview.pixels()),
        base64(&png)
    );
    Ok((entry, preview, png))
}

/// Appends the per-style references. Each must differ from the classic binary reference and from
/// every other style, otherwise the style is not exercised.
fn style_entries() -> Result<String, Box<dyn Error>> {
    let (_, baseline, _) = mode_entry(&mode_cases()?[0])?;
    let mut entries = Vec::new();
    let mut previews = vec![baseline];
    for case in &style_cases() {
        let (entry, preview, _) = style_entry(case)?;
        if previews
            .iter()
            .any(|earlier| earlier.pixels() == preview.pixels())
        {
            return Err(format!(
                "{}: style output must differ from earlier outputs",
                case.name
            )
            .into());
        }
        previews.push(preview);
        entries.push(entry);
    }
    Ok(entries.join(","))
}

/// One text reference, rendered directly by `ArtworkTextTask` and `Renderer::text_rows`. The
/// invert case uses colours and source colour mode; its text must match the same settings with
/// default colours, so colour is shown to have no effect while tone applies.
struct TextCase {
    name: &'static str,
    mode: RenderMode,
    sampling: Sampling,
    ramp: Option<&'static str>,
    style: RenderStyle,
}

fn text_cases() -> Result<[TextCase; 6], Box<dyn Error>> {
    let defaults = RenderStyle::default();
    Ok([
        TextCase {
            name: "classic",
            mode: RenderMode::Binary,
            sampling: Sampling::Classic,
            ramp: None,
            style: defaults,
        },
        TextCase {
            name: "ascii_default",
            mode: RenderMode::Ascii(Ramp::DEFAULT_ASCII),
            sampling: Sampling::Average,
            ramp: None,
            style: defaults,
        },
        TextCase {
            name: "ascii_custom",
            mode: RenderMode::Ascii(Ramp::new(CUSTOM_RAMP)?),
            sampling: Sampling::Average,
            ramp: Some(CUSTOM_RAMP),
            style: defaults,
        },
        TextCase {
            name: "block",
            mode: RenderMode::Block,
            sampling: Sampling::Average,
            ramp: None,
            style: defaults,
        },
        TextCase {
            name: "invert_coloured",
            mode: RenderMode::Binary,
            sampling: Sampling::Classic,
            ramp: None,
            style: RenderStyle {
                foreground: STYLE_FOREGROUND,
                background: STYLE_BACKGROUND,
                color: ColorMode::Source,
                invert: true,
                ..defaults
            },
        },
        TextCase {
            name: "contrast_raised",
            mode: RenderMode::Ascii(Ramp::DEFAULT_ASCII),
            sampling: Sampling::Average,
            ramp: None,
            style: RenderStyle {
                contrast: 100,
                ..defaults
            },
        },
    ])
}

/// Renders the text of one case with `style` through `ArtworkTextTask` and checks it against
/// `Renderer::text_rows` over the whole grid, so the task and the direct path must agree.
fn text_bytes(case: &TextCase, style: RenderStyle) -> Result<Vec<u8>, Box<dyn Error>> {
    let pixels = rgba(MODE_WIDTH, MODE_HEIGHT);
    let dimensions = Dimensions::new(MODE_WIDTH, MODE_HEIGHT)?;
    let decoded = DecodedSource::from_rgba(dimensions, pixels, DecodeLimits::default())?;
    let mut sampler = Sampler::new(Arc::new(decoded));
    let renderer = Renderer::new()?;
    let grid = match case.sampling {
        Sampling::Classic => sampler.sample(MODE_CELL, Sampling::Classic)?,
        Sampling::Average => {
            let geometry = renderer.aspect_geometry(MODE_CELL, MODE_FONT)?;
            sampler.sample_average_rect(MODE_CELL, u32::from(geometry.source_cell_height))?
        }
    };
    let options = RenderOptions {
        cell_size: MODE_CELL,
        font_size: MODE_FONT,
        threshold: MODE_THRESHOLD,
        sampling: case.sampling,
        mode: case.mode,
        style,
        ..RenderOptions::default()
    };
    let direct = renderer.text_rows(&grid, options, 0, grid.height())?;
    let task = native_text(grid, options)?;
    if direct != task {
        return Err(format!("{}: ArtworkTextTask differs from text_rows", case.name).into());
    }
    Ok(task)
}

/// Drains a full `ArtworkTextTask` in one advance, the same way `native_export` drains PNG.
fn native_text(grid: Arc<SampleGrid>, options: RenderOptions) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut task = ArtworkTextTask::new(
        grid,
        Renderer::new()?,
        options,
        EXPORT_GENERATION,
        EXPORT_LIMITS,
    )?;
    task.advance(EXPORT_GENERATION, u64::MAX)?;
    let mut bytes = Vec::new();
    for chunk in task.drain_chunks()? {
        bytes.extend(chunk);
    }
    for chunk in task.finish(EXPORT_GENERATION)? {
        bytes.extend(chunk);
    }
    Ok(bytes)
}

/// Returns the JSON entry and the text bytes of one case. The case's text must equal its text
/// with default colours and colour mode, proving colour has no effect.
fn text_entry(case: &TextCase) -> Result<(String, Vec<u8>), Box<dyn Error>> {
    let text = text_bytes(case, case.style)?;
    let defaults = RenderStyle::default();
    let uncoloured = RenderStyle {
        foreground: defaults.foreground,
        background: defaults.background,
        color: defaults.color,
        ..case.style
    };
    if text_bytes(case, uncoloured)? != text {
        return Err(format!("{}: colour changes the text", case.name).into());
    }
    let pixels = rgba(MODE_WIDTH, MODE_HEIGHT);
    let source = base64(&pixels);
    let (mode, ramp) = match case.mode {
        RenderMode::Binary => ("binary", "null".to_owned()),
        RenderMode::Ascii(_) => (
            "ascii",
            format!("\"{}\"", case.ramp.unwrap_or(Ramp::DEFAULT_ASCII.as_str())),
        ),
        RenderMode::Block => ("block", "null".to_owned()),
    };
    let sampling = match case.sampling {
        Sampling::Classic => "classic",
        Sampling::Average => "average",
    };
    let color_mode = match case.style.color {
        ColorMode::Monochrome => "monochrome",
        ColorMode::Source => "source",
    };
    let entry = format!(
        r#"{{"name":"{}","mode":"{mode}","ramp":{ramp},"width":{MODE_WIDTH},"height":{MODE_HEIGHT},"cellSize":{MODE_CELL},"fontSize":{MODE_FONT},"threshold":{MODE_THRESHOLD},"sampling":"{sampling}","foreground":"{}","background":"{}","colorMode":"{color_mode}","contrast":{},"invert":{},"source":"{source}","text":"{}"}}"#,
        case.name,
        colour_hex(case.style.foreground),
        colour_hex(case.style.background),
        case.style.contrast,
        case.style.invert,
        base64(&text)
    );
    Ok((entry, text))
}

/// Appends the text references. Each must differ from every other, otherwise the case does not
/// exercise its mode or tone.
fn text_entries() -> Result<String, Box<dyn Error>> {
    let mut entries = Vec::new();
    let mut texts: Vec<Vec<u8>> = Vec::new();
    for case in &text_cases()? {
        let (entry, text) = text_entry(case)?;
        if texts.contains(&text) {
            return Err(format!("{}: text must differ from earlier cases", case.name).into());
        }
        texts.push(text);
        entries.push(entry);
    }
    Ok(entries.join(","))
}

/// Closes the JSON object with the `text` array, the last field of the reference.
fn text_section() -> Result<String, Box<dyn Error>> {
    Ok(format!("],\"text\":[{}]}}", text_entries()?))
}

/// Writes every file `crates/desktop/src/smoke/e2e.rs` reads, from the same deterministic sources
/// and native renders as the JSON reference, so the desktop smoke needs no hand-made fixtures.
fn write_e2e_fixtures(dir: &Path, fixtures: &[Fixture]) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(dir)?;
    let write = |name: &str, bytes: &[u8]| -> Result<(), Box<dyn Error>> {
        fs::write(dir.join(name), bytes)?;
        Ok(())
    };
    let png = fixtures
        .iter()
        .find(|fixture| fixture.name == "png")
        .ok_or("png fixture exists")?;
    let (_, _, ui_pixels) = ui_preview(&png.bytes)?;
    let (_, export) = preview_of(decode(&png.bytes, DecodeLimits::default())?)?;
    write("png23x17.png", &png.bytes)?;
    write("png23x17.export.png", &export)?;
    write("png23x17.ui-preview.rgba", &ui_pixels)?;

    let large = DynamicImage::ImageRgba8(
        ImageBuffer::from_raw(LARGE_SIDE, LARGE_SIDE, large_rgba())
            .ok_or("large pixels match the dimensions")?,
    );
    write("large1200.png", &encode(&large, ImageFormat::Png)?)?;
    write("large1200.export.png", &large_export(Sampling::Classic)?)?;

    let mode_pixels = DynamicImage::ImageRgba8(
        ImageBuffer::from_raw(MODE_WIDTH, MODE_HEIGHT, rgba(MODE_WIDTH, MODE_HEIGHT))
            .ok_or("mode pixels match the dimensions")?,
    );
    write("mode64x40.png", &encode(&mode_pixels, ImageFormat::Png)?)?;
    let ascii = mode_cases()?
        .into_iter()
        .find(|case| case.name == "ascii_custom")
        .ok_or("ascii_custom case exists")?;
    let (_, preview, export) = mode_entry(&ascii)?;
    write("mode64x40.ascii-custom.preview.rgba", preview.pixels())?;
    write("mode64x40.ascii-custom.export.png", &export)?;

    for case in &style_cases() {
        let (_, preview, export) = style_entry(case)?;
        write(
            &format!("style64x40.{}.preview.rgba", case.name),
            preview.pixels(),
        )?;
        write(&format!("style64x40.{}.export.png", case.name), &export)?;
    }
    for case in &text_cases()? {
        let (_, text) = text_entry(case)?;
        write(&format!("text64x40.{}.export.txt", case.name), &text)?;
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let path = arguments
        .first()
        .cloned()
        .ok_or("usage: reference_fixtures <output.json> [<ui-preview.rgba> [<e2e-fixture-dir>]]")?;
    let mut json = String::from("{\"cases\":[");
    for (index, case) in CASES.iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        let pixels = rgba(case.width, case.height);
        let source = base64(&pixels);
        let frames = render_case(case, pixels)?;
        let sampling = match case.sampling {
            Sampling::Classic => "classic",
            Sampling::Average => "average",
        };
        write!(
            json,
            "{{\"name\":\"{}\",\"width\":{},\"height\":{},\"cellSize\":{},\"fontSize\":{},\"sampling\":\"{sampling}\",\"source\":\"{source}\",\"variants\":[",
            case.name, case.width, case.height, case.cell_size, case.font_size
        )?;
        for (position, (threshold, frame, png)) in frames.iter().enumerate() {
            if position > 0 {
                json.push(',');
            }
            write!(
                json,
                "{{\"threshold\":{threshold},\"previewWidth\":{},\"previewHeight\":{},\"preview\":\"{}\",\"export\":\"{}\"}}",
                frame.width(),
                frame.height(),
                base64(frame.pixels()),
                base64(png)
            )?;
        }
        json.push_str("]}");
    }
    json.push_str("],\"encoded\":[");
    let fixtures = encoded_fixtures()?;
    for (index, fixture) in fixtures.iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        json.push_str(&encoded_entry(fixture)?);
    }
    json.push_str("],\"large\":[");
    for (index, (name, sampling)) in [
        ("classic", Sampling::Classic),
        ("average", Sampling::Average),
    ]
    .into_iter()
    .enumerate()
    {
        if index > 0 {
            json.push(',');
        }
        let png = large_export(sampling)?;
        eprintln!("{name}: large export {} bytes", png.len());
        write!(
            json,
            "{{\"name\":\"{name}\",\"width\":{LARGE_SIDE},\"height\":{LARGE_SIDE},\"cellSize\":5,\"fontSize\":10,\"sampling\":\"{name}\",\"threshold\":128,\"export\":\"{}\"}}",
            base64(&png)
        )?;
    }
    json.push_str(&animation_sections()?);
    for (index, rejected) in rejected_fixtures(&fixtures)?.iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        write!(
            json,
            "{{\"name\":\"{}\",\"code\":\"{}\",\"bytes\":\"{}\"}}",
            rejected.name,
            rejected.code,
            base64(&rejected.bytes)
        )?;
    }
    json.push_str("],\"modes\":[");
    json.push_str(&mode_entries()?);
    json.push_str("],\"styles\":[");
    json.push_str(&style_entries()?);
    json.push_str(&text_section()?);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, json)?;
    if let Some(dir) = arguments.get(2) {
        write_e2e_fixtures(dir, &fixtures)?;
        eprintln!("desktop smoke fixtures written to {}", dir.display());
    }
    if let Some(preview_path) = arguments.get(1) {
        let png = fixtures
            .iter()
            .find(|fixture| fixture.name == "png")
            .ok_or("png fixture exists")?;
        let (width, height, pixels) = ui_preview(&png.bytes)?;
        fs::write(preview_path, &pixels)?;
        eprintln!("ui preview {width}x{height}: {} bytes", pixels.len());
    }
    Ok(())
}
