//! Reproducible stage timings for the core processing pipeline.

use std::io::Cursor;
use std::mem::size_of;
use std::time::Instant;

use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
use image_to_binary_core::{Cancellation, Generation, RenderMode, Sampling};
use image_to_binary_export::{BrowserChunkSink, ExportError, PngExportPlan};
use image_to_binary_processing::{decode, DecodeLimits, RenderOptions, Renderer, Sampler};

const FONT_SIZE: u8 = 20;
const CELL_SIZE: u8 = 20;
const OUTPUT_LIMIT: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy)]
struct Case {
    label: &'static str,
    width: u32,
    height: u32,
}

fn cases() -> [Case; 3] {
    [
        Case {
            label: "1MP",
            width: 1_000,
            height: 1_000,
        },
        Case {
            label: "12MP",
            width: 4_000,
            height: 3_000,
        },
        Case {
            label: "50MP",
            width: 10_000,
            height: 5_000,
        },
    ]
}

fn fixture(case: Case) -> Vec<u8> {
    let image = ImageBuffer::from_fn(case.width, case.height, |x, y| {
        let checker = ((x / 32) + (y / 32)) % 2;
        let gradient = u8::try_from((x + y) % 256).unwrap_or_default();
        if checker == 0 {
            Rgba([gradient, 255_u8.saturating_sub(gradient), 64, 255])
        } else {
            Rgba([16, gradient, 255_u8.saturating_sub(gradient), 255])
        }
    });
    let mut encoded = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut encoded, ImageFormat::Png)
        .expect("fixture encoding");
    encoded.into_inner()
}

fn timed<T>(operation: impl FnOnce() -> T) -> (T, f64) {
    let started = Instant::now();
    let value = operation();
    (value, started.elapsed().as_secs_f64() * 1_000.0)
}

fn options() -> RenderOptions {
    RenderOptions {
        cell_size: CELL_SIZE,
        font_size: FONT_SIZE,
        threshold: 128,
        sampling: Sampling::Classic,
        mode: RenderMode::Binary,
        max_output_bytes: OUTPUT_LIMIT,
    }
}

fn row_provider<'a>(
    renderer: &'a mut Renderer,
    grid: &'a image_to_binary_processing::SampleGrid,
    options: RenderOptions,
    frame: (u32, u32),
) -> impl FnMut(u32) -> Result<Vec<u8>, ExportError> + 'a {
    let tile_height = frame.1 / grid.height();
    let row_bytes = usize::try_from(u64::from(frame.0) * 4).expect("row bytes");
    let mut batch = Vec::new();
    move |row| {
        let source_row = row / tile_height;
        let row_in_batch = row % tile_height;
        if row_in_batch == 0 {
            batch = renderer
                .render_rows(grid, options, source_row, 1)
                .map_err(|_| {
                    ExportError::Contract(image_to_binary_core::ContractError::InvalidSettings(
                        "render row",
                    ))
                })?;
        }
        let start = usize::try_from(row_in_batch).unwrap() * row_bytes;
        Ok(batch[start..start + row_bytes].to_vec())
    }
}

fn export_native(
    renderer: &mut Renderer,
    grid: &image_to_binary_processing::SampleGrid,
    options: RenderOptions,
    frame: (u32, u32),
) -> (f64, u64) {
    let plan = PngExportPlan::new(frame.0, frame.1, OUTPUT_LIMIT).expect("export plan");
    let mut path = std::env::temp_dir();
    path.push(format!(
        "image-to-binary-core-bench-{}-{}.png",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let mut output = std::fs::File::create_new(&path).expect("native benchmark file");
    let cancellation = Cancellation::new(Generation::new(0));
    let ((), elapsed) = timed(|| {
        plan.encode(
            &mut output,
            &cancellation,
            Generation::new(0),
            row_provider(renderer, grid, options, frame),
        )
        .expect("native PNG export");
    });
    drop(output);
    let bytes = std::fs::metadata(&path)
        .expect("native benchmark metadata")
        .len();
    std::fs::remove_file(&path).expect("native benchmark cleanup");
    (elapsed, bytes)
}

fn export_browser(
    renderer: &mut Renderer,
    grid: &image_to_binary_processing::SampleGrid,
    options: RenderOptions,
    frame: (u32, u32),
) -> (f64, usize, usize) {
    let plan = PngExportPlan::new(frame.0, frame.1, OUTPUT_LIMIT).expect("export plan");
    let mut sink = BrowserChunkSink::new(64 * 1024, OUTPUT_LIMIT).expect("chunk sink");
    let cancellation = Cancellation::new(Generation::new(1));
    let ((), elapsed) = timed(|| {
        plan.encode(
            &mut sink,
            &cancellation,
            Generation::new(1),
            row_provider(renderer, grid, options, frame),
        )
        .expect("browser PNG export");
    });
    let chunks = sink.into_chunks().expect("browser chunks");
    let bytes = chunks.iter().map(Vec::len).sum();
    (elapsed, bytes, chunks.len())
}

fn run(case: Case) {
    let encoded = fixture(case);
    let (source, decode_ms) =
        timed(|| decode(&encoded, DecodeLimits::default()).expect("decode fixture"));
    let encoded_bytes = encoded.len();
    let decoded_bytes = source.pixels().len();
    drop(encoded);
    let mut sampler = Sampler::new(std::sync::Arc::new(source));
    let (grid, sample_ms) = timed(|| {
        sampler
            .sample(CELL_SIZE, Sampling::Classic)
            .expect("sample grid")
    });
    let grid_bytes = grid.len() * size_of::<image_to_binary_processing::SampleCell>();
    let mut renderer = Renderer::new().expect("renderer");
    let render_options = options();
    let (frame, render_ms) = timed(|| {
        renderer
            .render(&grid, render_options)
            .expect("render frame")
    });
    let full_bytes = frame.pixels().len();
    drop(frame);
    let (mut cold, cold_font_ms) = timed(|| Renderer::new().expect("cold renderer"));
    let (cold_preview, cold_ms) = timed(|| {
        cold.render_preview(&grid, render_options)
            .expect("cold preview")
    });
    let preview_bytes = cold_preview.pixels().len();
    drop(cold_preview);
    let (_, warm_ms) = timed(|| {
        renderer
            .render_preview(&grid, render_options)
            .expect("warm preview")
    });
    let frame_dimensions = renderer
        .frame_dimensions(&grid, render_options)
        .expect("frame dimensions");
    let native = export_native(&mut renderer, &grid, render_options, frame_dimensions);
    let browser = export_browser(&mut renderer, &grid, render_options, frame_dimensions);
    println!("case={} source={}x{} encoded={} decoded_rgba={} decode_ms={decode_ms:.3} sample_ms={sample_ms:.3} grid_cells={} grid_payload={} render_ms={render_ms:.3} full_rgba={} preview_rgba={} preview_font_ms={cold_font_ms:.3} preview_cold_ms={cold_ms:.3} preview_warm_ms={warm_ms:.3} native_export_ms={:.3} native_png={} browser_export_ms={:.3} browser_png={} browser_chunks={} row_batch_payload={} targets_ms=100/250", case.label, case.width, case.height, encoded_bytes, decoded_bytes, grid.len(), grid_bytes, full_bytes, preview_bytes, native.0, native.1, browser.0, browser.1, browser.2, usize::try_from(u64::from(case.width) * u64::from(CELL_SIZE) * 4).unwrap_or(0));
}

fn main() {
    let selected = std::env::args().nth(1);
    if let Some(value) = selected.as_deref() {
        if value != "all" && !["1", "12", "50", "1MP", "12MP", "50MP"].contains(&value) {
            eprintln!("usage: core_bench [all|1|12|50]");
            std::process::exit(2);
        }
    }
    for case in cases() {
        if selected.as_deref().is_some_and(|value| {
            value != "all" && value != case.label && value != case.label.trim_end_matches("MP")
        }) {
            continue;
        }
        run(case);
    }
}
