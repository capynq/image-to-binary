//! Writes native preview references and generated encoded fixtures (JSON, base64)
//! for the WASM binding tests.

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

use image::codecs::gif::GifEncoder;
use image::{Delay, DynamicImage, Frame, ImageBuffer, ImageFormat, Luma, Rgba};
use image_to_binary_core::{ContractError, Dimensions, Sampling};
use image_to_binary_processing::{
    decode, DecodeError, DecodeLimits, DecodedSource, RenderOptions, RenderedFrame, Renderer,
    Sampler,
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

fn render_case(case: &Case, pixels: Vec<u8>) -> Result<Vec<(u8, RenderedFrame)>, Box<dyn Error>> {
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
        frames.push((threshold, preview));
    }
    if frames.len() > 1 && frames[0].1.pixels() == frames[1].1.pixels() {
        return Err(format!("{}: threshold variants must differ", case.name).into());
    }
    Ok(frames)
}

const PREVIEW_CELL: u8 = 5;
const PREVIEW_FONT: u8 = 10;
const PREVIEW_THRESHOLD: u8 = 128;
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

fn preview_of(source: DecodedSource) -> Result<RenderedFrame, Box<dyn Error>> {
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
    Ok(renderer.render_preview(&grid, options)?)
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
    let preview = preview_of(source)?;
    Ok(format!(
        r#"{{"name":"{}","width":{width},"height":{height},"channels":{channels},"previewWidth":{},"previewHeight":{},"bytes":"{}","source":"{}","preview":"{}"}}"#,
        fixture.name,
        preview.width(),
        preview.height(),
        base64(&fixture.bytes),
        normalized,
        base64(preview.pixels())
    ))
}

fn main() -> Result<(), Box<dyn Error>> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: reference_fixtures <output.json>")?;
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
        for (position, (threshold, frame)) in frames.iter().enumerate() {
            if position > 0 {
                json.push(',');
            }
            write!(
                json,
                "{{\"threshold\":{threshold},\"previewWidth\":{},\"previewHeight\":{},\"preview\":\"{}\"}}",
                frame.width(),
                frame.height(),
                base64(frame.pixels())
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
    json.push_str("],\"rejected\":[");
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
    json.push_str("]}");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, json)?;
    Ok(())
}
