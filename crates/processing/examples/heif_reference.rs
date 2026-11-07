//! Writes native HEIC/AVIF references for the browser acceptance probe. For every fixture in
//! `tests/fixtures/heif`, the output directory receives the normalized RGBA8 pixels, the classic
//! PNG export at the UI defaults, and an `index.json` describing both.

use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use image_to_binary_adapters::{ArtworkPngTask, PngTaskLimits};
use image_to_binary_core::{Generation, Sampling};
use image_to_binary_processing::{
    decode, DecodeLimits, RenderOptions, Renderer, SampleGrid, Sampler,
};

const UI_CELL: u8 = 12;
const UI_FONT: u8 = 16;
const UI_THRESHOLD: u8 = 128;
const EXPORT_GENERATION: Generation = Generation::new(0);
const EXPORT_LIMITS: PngTaskLimits =
    PngTaskLimits::new(256 * 1024 * 1024, 1024 * 1024, 256 * 1024 * 1024);

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

fn main() -> Result<(), Box<dyn Error>> {
    let out_dir = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: heif_reference <output-dir>")?,
    );
    let fixtures_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/heif");
    fs::create_dir_all(&out_dir)?;

    let mut names = Vec::new();
    for entry in fs::read_dir(&fixtures_dir)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name.strip_suffix(".heic").is_some() || name.strip_suffix(".avif").is_some() {
            names.push(name);
        }
    }
    names.sort();

    let mut entries = Vec::new();
    for name in &names {
        let bytes = fs::read(fixtures_dir.join(name))?;
        let source = decode(&bytes, DecodeLimits::default())?;
        let width = source.metadata().dimensions().width();
        let height = source.metadata().dimensions().height();
        let channels = source.metadata().channels();
        let rgba_name = format!("{name}.rgba");
        let png_name = format!("{name}.png");
        fs::write(out_dir.join(&rgba_name), source.pixels())?;

        let mut sampler = Sampler::new(Arc::new(source));
        let grid = sampler.sample(UI_CELL, Sampling::Classic)?;
        let options = RenderOptions {
            cell_size: UI_CELL,
            font_size: UI_FONT,
            threshold: UI_THRESHOLD,
            sampling: Sampling::Classic,
            ..RenderOptions::default()
        };
        let png = native_export(&grid, options)?;
        fs::write(out_dir.join(&png_name), &png)?;
        eprintln!(
            "{name}: {width}x{height} channels {channels}, export {} bytes",
            png.len()
        );
        entries.push(format!(
            "{{\"name\":\"{name}\",\"width\":{width},\"height\":{height},\"channels\":{channels},\"rgbaFile\":\"{rgba_name}\",\"pngFile\":\"{png_name}\",\"exportSettings\":{{\"cellSize\":{UI_CELL},\"fontSize\":{UI_FONT},\"threshold\":{UI_THRESHOLD},\"sampling\":\"classic\"}}}}"
        ));
    }
    fs::write(
        out_dir.join("index.json"),
        format!("{{\"fixtures\":[{}]}}\n", entries.join(",")),
    )?;
    Ok(())
}
