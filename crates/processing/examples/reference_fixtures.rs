//! Writes native preview references (JSON, base64 RGBA8) for the WASM binding tests.

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use image_to_binary_core::{Dimensions, Sampling};
use image_to_binary_processing::{
    DecodeLimits, DecodedSource, RenderOptions, RenderedFrame, Renderer, Sampler,
};

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
    json.push_str("]}");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, json)?;
    Ok(())
}
