//! Writes browser import inputs and their native references for the format coverage probe.
//!
//! Each preview case receives the native decoded RGBA8 pixels (`references/`) and the
//! native classic PNG export at the UI defaults (cell 12, font 16, threshold 128,
//! `exports/`). A negative control is what a wrong interpretation would produce (orientation
//! ignored, the other page or frame, alpha forced opaque, or a 16-bit sample reduced to its
//! high byte instead of the native rounding). `controlExportDiffers` and `controlPixelsDiffer` record whether each control
//! is visible in the export and in the pixels. Limit cases carry only the crafted header
//! files and their native outcomes under the default and the 16 MP limits. The output
//! directory receives `inputs/`, `references/`, `exports/` and `manifest.json`.

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::io::{Cursor, Seek, Write};
use std::path::PathBuf;
use std::sync::Arc;

use image::codecs::bmp::BmpEncoder;
use image::codecs::gif::GifEncoder;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;
use image::{
    DynamicImage, ExtendedColorType, Frame, ImageBuffer, ImageEncoder, ImageError, ImageFormat,
    Rgba,
};
use image_to_binary_adapters::{ArtworkPngTask, PngTaskLimits};
use image_to_binary_core::{ContractError, Dimensions, Generation, ResourceLimits, Sampling};
use image_to_binary_processing::{
    decode, DecodeError, DecodeLimits, DecodedSource, RenderOptions, Renderer, SampleGrid, Sampler,
};
use tiff::encoder::{colortype, TiffEncoder};
use tiff::tags::Tag;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const WIDTH: u32 = 120;
const HEIGHT: u32 = 96;
const UI_CELL: u8 = 12;
const UI_FONT: u8 = 16;
const UI_THRESHOLD: u8 = 128;
const EXPORT_GENERATION: Generation = Generation::new(0);
const EXPORT_LIMITS: PngTaskLimits =
    PngTaskLimits::new(256 * 1024 * 1024, 1024 * 1024, 256 * 1024 * 1024);
const STRIPES16: [u16; 6] = [0xFF00, 0x8000, 0x7F00, 0x80FF, 0x0000, 0xFFFF];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Codec {
    Jpeg,
    Png,
    WebP,
    Bmp,
    Tiff,
    Gif,
}

impl Codec {
    const ALL: [Self; 6] = [
        Self::Jpeg,
        Self::Png,
        Self::WebP,
        Self::Bmp,
        Self::Tiff,
        Self::Gif,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::Jpeg => "jpeg",
            Self::Png => "png",
            Self::WebP => "webp",
            Self::Bmp => "bmp",
            Self::Tiff => "tiff",
            Self::Gif => "gif",
        }
    }

    const fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::WebP => "webp",
            Self::Bmp => "bmp",
            Self::Tiff => "tiff",
            Self::Gif => "gif",
        }
    }
}

struct Control {
    note: &'static str,
    png: Vec<u8>,
    pixels: Vec<u8>,
}

struct Entry {
    id: String,
    group: &'static str,
    property: &'static str,
    codec: Codec,
    bytes: Vec<u8>,
    header: (u32, u32),
    outcome: &'static str,
    decoded: Option<(u32, u32)>,
    reference: Option<Vec<u8>>,
    export: Option<Vec<u8>>,
    control: Option<Control>,
    control_export_differs: Option<bool>,
    control_pixels_differ: Option<bool>,
    lowered: Option<&'static str>,
}

fn byte(value: u32) -> u8 {
    u8::try_from(value).expect("channel fits in u8")
}

fn rgb_pixels() -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            pixels.extend_from_slice(&[
                byte(x * 255 / (WIDTH - 1)),
                byte(y * 255 / (HEIGHT - 1)),
                byte((x * 37 + y * 91) % 256),
            ]);
        }
    }
    pixels
}

fn grey_pixels() -> Vec<u8> {
    (0..HEIGHT)
        .flat_map(|_| (0..WIDTH).map(|x| byte(x * 255 / (WIDTH - 1))))
        .collect()
}

fn with_alpha(rgb: &[u8], alpha: impl Fn(u32, u32) -> u8) -> Vec<u8> {
    let mut pixels = Vec::new();
    for (index, chunk) in rgb.chunks_exact(3).enumerate() {
        let index = u32::try_from(index).expect("pixel index fits");
        let (x, y) = (index % WIDTH, index / WIDTH);
        pixels.extend_from_slice(&[chunk[0], chunk[1], chunk[2], alpha(x, y)]);
    }
    pixels
}

fn partial_alpha(x: u32, y: u32) -> u8 {
    byte(55 + (x * 3 + y * 5) % 200)
}

fn binary_alpha(x: u32, y: u32) -> u8 {
    if (x / 12 + y / 12) % 2 == 0 {
        255
    } else {
        0
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
    pixels: &[u8],
    orientation: Option<u8>,
) -> Result<()> {
    let mut image = encoder.new_image::<C>(WIDTH, HEIGHT)?;
    if let Some(orientation) = orientation {
        image
            .encoder()
            .write_tag(Tag::Orientation, u16::from(orientation))?;
    }
    image.write_data(pixels)?;
    Ok(())
}

fn rgba_of(color: ExtendedColorType, pixels: &[u8]) -> Result<Vec<u8>> {
    Ok(match color {
        ExtendedColorType::L8 => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        ExtendedColorType::Rgb8 => pixels
            .chunks_exact(3)
            .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255])
            .collect(),
        ExtendedColorType::Rgba8 => pixels.to_vec(),
        other => return Err(format!("unsupported colour type {other:?}").into()),
    })
}

fn encode_gif(frames: Vec<Vec<u8>>) -> Result<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    let mut built = Vec::new();
    for rgba in frames {
        let buffer = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(WIDTH, HEIGHT, rgba)
            .ok_or("frame pixels match the dimensions")?;
        built.push(Frame::new(buffer));
    }
    GifEncoder::new(&mut output).encode_frames(built)?;
    Ok(output.into_inner())
}

fn encode(
    codec: Codec,
    color: ExtendedColorType,
    pixels: &[u8],
    orientation: Option<u8>,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    match codec {
        Codec::Jpeg => {
            let mut encoder = JpegEncoder::new_with_quality(&mut output, 100);
            if let Some(orientation) = orientation {
                encoder.set_exif_metadata(exif_block(orientation))?;
            }
            encoder.write_image(pixels, WIDTH, HEIGHT, color)?;
        }
        Codec::Png => {
            let mut encoder = PngEncoder::new(&mut output);
            if let Some(orientation) = orientation {
                encoder.set_exif_metadata(exif_block(orientation))?;
            }
            encoder.write_image(pixels, WIDTH, HEIGHT, color)?;
        }
        Codec::WebP => {
            let mut encoder = WebPEncoder::new_lossless(&mut output);
            if let Some(orientation) = orientation {
                encoder.set_exif_metadata(exif_block(orientation))?;
            }
            encoder.write_image(pixels, WIDTH, HEIGHT, color)?;
        }
        Codec::Bmp => {
            BmpEncoder::new(&mut output).write_image(pixels, WIDTH, HEIGHT, color)?;
        }
        Codec::Tiff => {
            let mut encoder = TiffEncoder::new(Cursor::new(&mut output))?;
            match color {
                ExtendedColorType::L8 => {
                    write_tiff_page::<_, colortype::Gray8>(&mut encoder, pixels, orientation)?;
                }
                ExtendedColorType::Rgb8 => {
                    write_tiff_page::<_, colortype::RGB8>(&mut encoder, pixels, orientation)?;
                }
                ExtendedColorType::Rgba8 => {
                    write_tiff_page::<_, colortype::RGBA8>(&mut encoder, pixels, orientation)?;
                }
                other => return Err(format!("unsupported TIFF colour type {other:?}").into()),
            }
        }
        Codec::Gif => return encode_gif(vec![rgba_of(color, pixels)?]),
    }
    Ok(output)
}

fn two_page_tiff(first: &[u8], second: &[u8]) -> Result<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    {
        let mut encoder = TiffEncoder::new(&mut output)?;
        write_tiff_page::<_, colortype::RGB8>(&mut encoder, first, None)?;
        write_tiff_page::<_, colortype::RGB8>(&mut encoder, second, None)?;
    }
    Ok(output.into_inner())
}

/// The 24x16 TIFF base used by `format_matrix.rs`; the 120x96 base hits "inconsistent sizes" before the limit check.
fn tiff_limit_base() -> Result<Vec<u8>> {
    let mut pixels = Vec::new();
    for y in 0..16_u32 {
        for x in 0..24_u32 {
            pixels.extend_from_slice(&[
                byte(x * 255 / 23),
                byte(y * 255 / 15),
                byte((x * 37 + y * 91) % 256),
            ]);
        }
    }
    let mut output = Cursor::new(Vec::new());
    {
        let mut encoder = TiffEncoder::new(&mut output)?;
        encoder.write_image::<colortype::RGB8>(24, 16, &pixels)?;
    }
    Ok(output.into_inner())
}

fn sixteen_bit_pixels() -> Vec<u16> {
    let mut pixels = Vec::new();
    for _ in 0..HEIGHT {
        for x in 0..WIDTH {
            let value = STRIPES16[usize::try_from(x / 20).expect("stripe index fits")];
            pixels.extend_from_slice(&[value, value, value]);
        }
    }
    pixels
}

fn png16(pixels: &[u16]) -> Result<Vec<u8>> {
    let image = DynamicImage::ImageRgb16(
        ImageBuffer::from_raw(WIDTH, HEIGHT, pixels.to_vec())
            .ok_or("16-bit pixels match the dimensions")?,
    );
    let mut output = Cursor::new(Vec::new());
    image.write_to(&mut output, ImageFormat::Png)?;
    Ok(output.into_inner())
}

fn tiff16(pixels: &[u16]) -> Result<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    {
        let mut encoder = TiffEncoder::new(&mut output)?;
        encoder.write_image::<colortype::RGB16>(WIDTH, HEIGHT, pixels)?;
    }
    Ok(output.into_inner())
}

/// Keeps only the high byte of each 16-bit sample, the wrong reading of the native rounding.
fn truncated_sixteen_bit_source(pixels: &[u16]) -> Result<DecodedSource> {
    let mut rgba = Vec::new();
    for value in pixels.chunks_exact(3) {
        let alt = byte(u32::from(value[0]) >> 8);
        rgba.extend_from_slice(&[alt, alt, alt, 255]);
    }
    let dimensions = Dimensions::new(WIDTH, HEIGHT)?;
    Ok(DecodedSource::from_rgba(
        dimensions,
        rgba,
        DecodeLimits::default(),
    )?)
}

fn opaque_source(source: &DecodedSource) -> Result<DecodedSource> {
    let dimensions = source.metadata().dimensions();
    let mut pixels = source.pixels().to_vec();
    for alpha in pixels.chunks_exact_mut(4) {
        alpha[3] = 255;
    }
    Ok(DecodedSource::from_rgba(
        dimensions,
        pixels,
        DecodeLimits::default(),
    )?)
}

fn browser_limits() -> Result<DecodeLimits> {
    Ok(DecodeLimits::new(
        ResourceLimits::new(16_000_000, 256 * 1024 * 1024)?,
        256 * 1024 * 1024,
        1024 * 1024 * 1024,
    )?)
}

fn ui_options() -> RenderOptions {
    RenderOptions {
        cell_size: UI_CELL,
        font_size: UI_FONT,
        threshold: UI_THRESHOLD,
        sampling: Sampling::Classic,
        ..RenderOptions::default()
    }
}

fn native_export(grid: &Arc<SampleGrid>, options: RenderOptions) -> Result<Vec<u8>> {
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

fn export_png(source: DecodedSource) -> Result<Vec<u8>> {
    let mut sampler = Sampler::new(Arc::new(source));
    let grid = sampler.sample(UI_CELL, Sampling::Classic)?;
    native_export(&grid, ui_options())
}

fn classify(result: &std::result::Result<DecodedSource, DecodeError>) -> &'static str {
    match result {
        Ok(_) => "preview",
        Err(DecodeError::Contract(ContractError::ResourceLimitExceeded)) => {
            "resource_limit_exceeded"
        }
        Err(DecodeError::Image(ImageError::Decoding(_))) => "decoding_error",
        Err(DecodeError::Image(ImageError::IoError(_))) => "io_error",
        Err(_) => "other_error",
    }
}

fn format_entry(
    id: String,
    property: &'static str,
    codec: Codec,
    bytes: Vec<u8>,
    control: Option<(&'static str, DecodedSource)>,
) -> Result<Entry> {
    let decoded = decode(&bytes, DecodeLimits::default());
    let outcome = classify(&decoded);
    let Ok(source) = decoded else {
        return Err(format!("{id}: native decode failed ({outcome})").into());
    };
    let dimensions = source.metadata().dimensions();
    let (width, height) = (dimensions.width(), dimensions.height());
    let reference = source.pixels().to_vec();
    let export = export_png(source)?;
    let control = match control {
        Some((note, control_source)) => {
            let pixels = control_source.pixels().to_vec();
            let png = export_png(control_source)?;
            Some(Control { note, png, pixels })
        }
        None => None,
    };
    let control_export_differs = control.as_ref().map(|control| control.png != export);
    let control_pixels_differ = control.as_ref().map(|control| control.pixels != reference);
    Ok(Entry {
        id,
        group: "format",
        property,
        codec,
        bytes,
        header: (WIDTH, HEIGHT),
        outcome,
        decoded: Some((width, height)),
        reference: Some(reference),
        export: Some(export),
        control,
        control_export_differs,
        control_pixels_differ,
        lowered: None,
    })
}

fn limit_entry(id: String, codec: Codec, bytes: Vec<u8>, header: (u32, u32)) -> Result<Entry> {
    let default_result = decode(&bytes, DecodeLimits::default());
    let outcome = classify(&default_result);
    drop(default_result);
    let lowered_result = decode(&bytes, browser_limits()?);
    let lowered = classify(&lowered_result);
    drop(lowered_result);
    Ok(Entry {
        id,
        group: "limit",
        property: "limit",
        codec,
        bytes,
        header,
        outcome,
        decoded: None,
        reference: None,
        export: None,
        control: None,
        control_export_differs: None,
        control_pixels_differ: None,
        lowered: Some(lowered),
    })
}

/// Rewrites the width and height fields of a base encoding, as in `format_matrix.rs`.
fn with_dimensions(codec: Codec, bytes: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    Ok(match codec {
        Codec::Jpeg => with_jpeg_dimensions(bytes, width, height)?,
        Codec::Png => with_png_dimensions(bytes, width, height),
        Codec::WebP => with_webp_dimensions(bytes, width, height),
        Codec::Bmp => with_bmp_dimensions(bytes, width, height)?,
        Codec::Tiff => with_tiff_dimensions(bytes, width, height),
        Codec::Gif => with_gif_dimensions(bytes, width, height)?,
    })
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

fn with_jpeg_dimensions(bytes: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let mut patched = bytes.to_vec();
    let marker = patched
        .windows(2)
        .position(|pair| pair == [0xFF, 0xC0])
        .ok_or("JPEG has a SOF0 marker")?;
    let height = u16::try_from(height)?;
    let width = u16::try_from(width)?;
    patched[marker + 5..marker + 7].copy_from_slice(&height.to_be_bytes());
    patched[marker + 7..marker + 9].copy_from_slice(&width.to_be_bytes());
    Ok(patched)
}

fn with_bmp_dimensions(bytes: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let mut patched = bytes.to_vec();
    patched[18..22].copy_from_slice(&i32::try_from(width)?.to_le_bytes());
    patched[22..26].copy_from_slice(&i32::try_from(height)?.to_le_bytes());
    Ok(patched)
}

fn with_gif_dimensions(bytes: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let mut patched = bytes.to_vec();
    patched[6..8].copy_from_slice(&u16::try_from(width)?.to_le_bytes());
    patched[8..10].copy_from_slice(&u16::try_from(height)?.to_le_bytes());
    Ok(patched)
}

fn with_tiff_dimensions(bytes: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut patched = bytes.to_vec();
    let ifd = usize::try_from(u32::from_le_bytes(
        bytes[4..8].try_into().expect("four bytes"),
    ))
    .expect("offset fits");
    let count = usize::from(u16::from_le_bytes(
        bytes[ifd..ifd + 2].try_into().expect("two bytes"),
    ));
    for index in 0..count {
        let entry = ifd + 2 + 12 * index;
        let tag = u16::from_le_bytes(bytes[entry..entry + 2].try_into().expect("two bytes"));
        let value = match tag {
            256 => width,
            257 => height,
            _ => continue,
        };
        let field = entry + 8;
        let kind = u16::from_le_bytes(bytes[entry + 2..entry + 4].try_into().expect("two bytes"));
        match kind {
            3 => patched[field..field + 2]
                .copy_from_slice(&u16::try_from(value).expect("fits").to_le_bytes()),
            _ => patched[field..field + 4].copy_from_slice(&value.to_le_bytes()),
        }
    }
    patched
}

fn with_webp_dimensions(bytes: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut patched = bytes.to_vec();
    let packed = u32::from_le_bytes(bytes[21..25].try_into().expect("four bytes"));
    let fields = (width - 1) | ((height - 1) << 14);
    let next = (packed & 0xF000_0000) | fields;
    patched[21..25].copy_from_slice(&next.to_le_bytes());
    patched
}

/// Two-frame APNG (RGBA8) with the first frame in IDAT, as `format_matrix.rs` writes it.
fn encode_apng(first: &[u8], second: &[u8]) -> Result<Vec<u8>> {
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

fn lossless_chunk(rgba: &[u8]) -> Result<Vec<u8>> {
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

fn riff_chunk(fourcc: [u8; 4], payload: &[u8]) -> Result<Vec<u8>> {
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

/// Animated WebP from lossless VP8L frames; `blend` is the ANMF flags byte (0 alpha-blend, 0x02 no blend).
fn animated_webp(frames: &[Vec<u8>], blend: u8) -> Result<Vec<u8>> {
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

#[allow(clippy::too_many_lines)]
fn main() -> Result<()> {
    let out = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: browser_matrix_reference <output-dir>")?,
    );
    fs::create_dir_all(out.join("inputs"))?;
    fs::create_dir_all(out.join("exports"))?;
    fs::create_dir_all(out.join("references"))?;

    let rgb = rgb_pixels();
    let grey = grey_pixels();
    let inverted: Vec<u8> = rgb.iter().map(|value| 255 - value).collect();
    let mut entries = Vec::new();

    for codec in Codec::ALL {
        let name = codec.name();
        let base = encode(codec, ExtendedColorType::Rgb8, &rgb, None)?;
        entries.push(format_entry(
            format!("{name}-rgb"),
            "rgb",
            codec,
            base.clone(),
            None,
        )?);

        entries.push(format_entry(
            format!("{name}-grey"),
            "grayscale",
            codec,
            encode(codec, ExtendedColorType::L8, &grey, None)?,
            None,
        )?);

        if codec != Codec::Jpeg {
            let alpha = if codec == Codec::Gif {
                with_alpha(&rgb, binary_alpha)
            } else {
                with_alpha(&rgb, partial_alpha)
            };
            let bytes = encode(codec, ExtendedColorType::Rgba8, &alpha, None)?;
            let decoded = decode(&bytes, DecodeLimits::default())?;
            let opaque = opaque_source(&decoded)?;
            entries.push(format_entry(
                format!("{name}-alpha"),
                "alpha",
                codec,
                bytes,
                Some(("alpha forced opaque", opaque)),
            )?);
        }

        if matches!(codec, Codec::Jpeg | Codec::Png | Codec::WebP | Codec::Tiff) {
            for orientation in 1..=8_u8 {
                let bytes = encode(codec, ExtendedColorType::Rgb8, &rgb, Some(orientation))?;
                let control = if orientation == 1 {
                    None
                } else {
                    let plain = encode(codec, ExtendedColorType::Rgb8, &rgb, Some(1))?;
                    Some((
                        "orientation tag ignored",
                        decode(&plain, DecodeLimits::default())?,
                    ))
                };
                entries.push(format_entry(
                    format!("{name}-orientation-{orientation}"),
                    "orientation",
                    codec,
                    bytes,
                    control,
                )?);
            }
        }
    }

    let tiff_pages = two_page_tiff(&rgb, &inverted)?;
    let second_page = encode(Codec::Tiff, ExtendedColorType::Rgb8, &inverted, None)?;
    entries.push(format_entry(
        "tiff-two-page".to_owned(),
        "multi_page",
        Codec::Tiff,
        tiff_pages,
        Some((
            "second page exported",
            decode(&second_page, DecodeLimits::default())?,
        )),
    )?);

    let rgba_first = rgba_of(ExtendedColorType::Rgb8, &rgb)?;
    let rgba_second = rgba_of(ExtendedColorType::Rgb8, &inverted)?;
    let gif_frames = encode_gif(vec![rgba_first, rgba_second.clone()])?;
    let gif_second = encode_gif(vec![rgba_second])?;
    entries.push(format_entry(
        "gif-two-frame".to_owned(),
        "multi_frame",
        Codec::Gif,
        gif_frames,
        Some((
            "second frame exported",
            decode(&gif_second, DecodeLimits::default())?,
        )),
    )?);

    let frame_one = rgba_of(ExtendedColorType::Rgb8, &rgb)?;
    let frame_two = rgba_of(ExtendedColorType::Rgb8, &inverted)?;
    let second_source = DecodedSource::from_rgba(
        Dimensions::new(WIDTH, HEIGHT)?,
        frame_two.clone(),
        DecodeLimits::default(),
    )?;
    entries.push(format_entry(
        "png-apng-first-frame".to_owned(),
        "multi_frame",
        Codec::Png,
        encode_apng(&frame_one, &frame_two)?,
        Some(("second frame exported", second_source)),
    )?);
    let still_one = lossless_chunk(&frame_one)?;
    let still_two = lossless_chunk(&frame_two)?;
    let second_source = DecodedSource::from_rgba(
        Dimensions::new(WIDTH, HEIGHT)?,
        frame_two.clone(),
        DecodeLimits::default(),
    )?;
    entries.push(format_entry(
        "webp-animated-no-blend".to_owned(),
        "multi_frame",
        Codec::WebP,
        animated_webp(&[still_one.clone(), still_two.clone()], 0x02)?,
        Some(("second frame exported", second_source)),
    )?);
    let second_source = DecodedSource::from_rgba(
        Dimensions::new(WIDTH, HEIGHT)?,
        frame_two,
        DecodeLimits::default(),
    )?;
    entries.push(format_entry(
        "webp-animated-alpha-blend".to_owned(),
        "multi_frame",
        Codec::WebP,
        animated_webp(&[still_one, still_two], 0)?,
        Some(("second frame exported", second_source)),
    )?);

    let sixteen = sixteen_bit_pixels();
    entries.push(format_entry(
        "png-16bit".to_owned(),
        "high_bit_depth",
        Codec::Png,
        png16(&sixteen)?,
        Some((
            "16-bit high byte only",
            truncated_sixteen_bit_source(&sixteen)?,
        )),
    )?);
    entries.push(format_entry(
        "tiff-16bit".to_owned(),
        "high_bit_depth",
        Codec::Tiff,
        tiff16(&sixteen)?,
        Some((
            "16-bit high byte only",
            truncated_sixteen_bit_source(&sixteen)?,
        )),
    )?);

    for codec in Codec::ALL {
        let base = if codec == Codec::Tiff {
            tiff_limit_base()?
        } else {
            encode(codec, ExtendedColorType::Rgb8, &rgb, None)?
        };
        for (label, width, height) in [
            ("50mp-accept", 10_000_u32, 5_000_u32),
            ("50mp-reject", 10_000, 5_001),
            ("16mp-accept", 4_000, 4_000),
            ("16mp-reject", 4_000, 4_001),
        ] {
            let bytes = with_dimensions(codec, &base, width, height)?;
            entries.push(limit_entry(
                format!("{}-limit-{label}", codec.name()),
                codec,
                bytes,
                (width, height),
            )?);
        }
    }

    let mut manifest = String::from("{\"cases\":[");
    for (index, entry) in entries.iter().enumerate() {
        let file = format!("{}.{}", entry.id, entry.codec.extension());
        fs::write(out.join("inputs").join(&file), &entry.bytes)?;
        let export_file = match &entry.export {
            Some(png) => {
                let name = format!("exports/{}.png", entry.id);
                fs::write(out.join(&name), png)?;
                format!("\"{name}\"")
            }
            None => "null".to_owned(),
        };
        let reference_file = match &entry.reference {
            Some(pixels) => {
                let name = format!("references/{}.rgba", entry.id);
                fs::write(out.join(&name), pixels)?;
                format!("\"{name}\"")
            }
            None => "null".to_owned(),
        };
        let (control_file, control_reference_file, control_note) = match &entry.control {
            Some(control) => {
                let name = format!("exports/{}.control.png", entry.id);
                fs::write(out.join(&name), &control.png)?;
                let reference = format!("references/{}.control.rgba", entry.id);
                fs::write(out.join(&reference), &control.pixels)?;
                (
                    format!("\"{name}\""),
                    format!("\"{reference}\""),
                    format!("\"{}\"", control.note),
                )
            }
            None => ("null".to_owned(), "null".to_owned(), "null".to_owned()),
        };
        let (native_width, native_height) = match entry.decoded {
            Some((width, height)) => (width.to_string(), height.to_string()),
            None => ("null".to_owned(), "null".to_owned()),
        };
        let control_export_differs = match entry.control_export_differs {
            Some(value) => value.to_string(),
            None => "null".to_owned(),
        };
        let control_pixels_differ = match entry.control_pixels_differ {
            Some(value) => value.to_string(),
            None => "null".to_owned(),
        };
        let lowered = match entry.lowered {
            Some(value) => format!("\"{value}\""),
            None => "null".to_owned(),
        };
        if index > 0 {
            manifest.push(',');
        }
        write!(
            manifest,
            "{{\"id\":\"{}\",\"group\":\"{}\",\"property\":\"{}\",\"codec\":\"{}\",\"file\":\"inputs/{file}\",\"headerWidth\":{},\"headerHeight\":{},\"nativeOutcome\":\"{}\",\"nativeWidth\":{native_width},\"nativeHeight\":{native_height},\"referenceFile\":{reference_file},\"exportFile\":{export_file},\"controlFile\":{control_file},\"controlReferenceFile\":{control_reference_file},\"controlNote\":{control_note},\"controlExportDiffers\":{control_export_differs},\"controlPixelsDiffer\":{control_pixels_differ},\"nativeLowered16Outcome\":{lowered}}}",
            entry.id,
            entry.group,
            entry.property,
            entry.codec.name(),
            entry.header.0,
            entry.header.1,
            entry.outcome,
        )?;
        eprintln!("{}: native {}", entry.id, entry.outcome);
    }
    manifest.push_str("]}\n");
    fs::write(out.join("manifest.json"), manifest)?;
    eprintln!("wrote {} cases to {}", entries.len(), out.display());
    Ok(())
}
