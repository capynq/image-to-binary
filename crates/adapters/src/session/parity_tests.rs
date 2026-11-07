//! Native parity: the shared session, driven the way the desktop worker drives it (advance with
//! `DEFAULT_PIXEL_BUDGET`, drain between advances), must produce the same bytes as direct renderer
//! output for the `reference_fixtures` cases.

use std::sync::Arc;

use image_to_binary_core::{Dimensions, Generation, Sampling};
use image_to_binary_processing::{
    DecodeLimits, DecodedSource, RenderOptions, RenderedFrame, Renderer, SampleGrid, Sampler,
};

use super::*;
use crate::ArtworkPngTask;

const CLASSIC_GOLDEN: u64 = 15_743_520_938_017_280_556;

struct Case {
    width: u32,
    height: u32,
    cell: u8,
    font: u8,
    sampling: Sampling,
}

/// The four cases of `crates/processing/examples/reference_fixtures.rs`, in the same order.
const CASES: [Case; 4] = [
    Case {
        width: 61,
        height: 47,
        cell: 7,
        font: 14,
        sampling: Sampling::Classic,
    },
    Case {
        width: 53,
        height: 71,
        cell: 6,
        font: 18,
        sampling: Sampling::Average,
    },
    Case {
        width: 3005,
        height: 10,
        cell: 5,
        font: 10,
        sampling: Sampling::Classic,
    },
    Case {
        width: 7,
        height: 2400,
        cell: 5,
        font: 12,
        sampling: Sampling::Average,
    },
];
const THRESHOLDS: [u8; 2] = [64, 192];

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
                pixels.push(u8::try_from(channel).unwrap());
            }
        }
    }
    pixels
}

fn source(case: &Case) -> DecodedSource {
    DecodedSource::from_rgba(
        Dimensions::new(case.width, case.height).unwrap(),
        rgba(case.width, case.height),
        DecodeLimits::default(),
    )
    .unwrap()
}

fn options(case: &Case, threshold: u8) -> RenderOptions {
    RenderOptions {
        cell_size: case.cell,
        font_size: case.font,
        threshold,
        sampling: case.sampling,
        ..RenderOptions::default()
    }
}

fn reference_grid(case: &Case, sampler: &mut Sampler, renderer: &Renderer) -> Arc<SampleGrid> {
    match case.sampling {
        Sampling::Classic => sampler.sample(case.cell, Sampling::Classic).unwrap(),
        Sampling::Average => {
            let rows = renderer
                .aspect_geometry(case.cell, case.font)
                .unwrap()
                .source_cell_height;
            sampler
                .sample_average_rect(case.cell, u32::from(rows))
                .unwrap()
        }
    }
}

fn reference_preview(case: &Case, threshold: u8) -> RenderedFrame {
    let mut renderer = Renderer::new().unwrap();
    let mut sampler = Sampler::new(Arc::new(source(case)));
    let grid = reference_grid(case, &mut sampler, &renderer);
    renderer
        .render_preview(&grid, options(case, threshold))
        .unwrap()
}

fn reference_png(case: &Case, threshold: u8) -> Vec<u8> {
    let renderer = Renderer::new().unwrap();
    let mut sampler = Sampler::new(Arc::new(source(case)));
    let grid = reference_grid(case, &mut sampler, &renderer);
    let mut task = ArtworkPngTask::new(
        grid,
        Renderer::new().unwrap(),
        options(case, threshold),
        Generation::new(0),
        EXPORT_LIMITS,
    )
    .unwrap();
    task.advance(Generation::new(0), u64::MAX).unwrap();
    let mut bytes = Vec::new();
    for chunk in task.drain_chunks().unwrap() {
        bytes.extend(chunk);
    }
    for chunk in task.finish(Generation::new(0)).unwrap() {
        bytes.extend(chunk);
    }
    bytes
}

fn settings(case: &Case, threshold: u8) -> PreviewSettings {
    PreviewSettings::new(case.cell, case.font, threshold, case.sampling).unwrap()
}

/// Drives a preview the way the desktop worker does and returns `(width, height, pixels)`.
fn session_preview(case: &Case, threshold: u8) -> (u32, u32, Vec<u8>) {
    let mut session = Session::new(source(case)).unwrap();
    session
        .start_preview(Generation::new(0), settings(case, threshold))
        .unwrap();
    while session.stage().unwrap() != Stage::Ready {
        session
            .advance(Generation::new(0), DEFAULT_PIXEL_BUDGET)
            .unwrap();
    }
    let mut frame = session.finish(Generation::new(0)).unwrap();
    let (width, height) = (frame.width(), frame.height());
    (width, height, frame.take_pixels().unwrap())
}

/// Drives an export the way the desktop worker does and returns the concatenated PNG.
fn session_png(case: &Case, threshold: u8) -> Vec<u8> {
    let mut session = Session::new(source(case)).unwrap();
    session
        .start_export(Generation::new(0), settings(case, threshold))
        .unwrap();
    let mut bytes = Vec::new();
    while session.stage().unwrap() != Stage::Ready {
        session
            .advance(Generation::new(0), DEFAULT_PIXEL_BUDGET)
            .unwrap();
        for chunk in session.drain_export(Generation::new(0)).unwrap() {
            bytes.extend(chunk);
        }
    }
    for chunk in session.finish_export(Generation::new(0)).unwrap() {
        bytes.extend(chunk);
    }
    bytes
}

fn fnv64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[test]
fn native_previews_match_the_reference_fixtures_byte_for_byte() {
    for case in &CASES {
        for threshold in THRESHOLDS {
            let expected = reference_preview(case, threshold);
            let (width, height, pixels) = session_preview(case, threshold);
            assert_eq!((width, height), (expected.width(), expected.height()));
            assert!(
                pixels == expected.pixels(),
                "preview differs at threshold {threshold}"
            );
        }
    }
}

#[test]
fn native_exports_match_the_reference_fixtures_byte_for_byte() {
    for case in &CASES {
        for threshold in THRESHOLDS {
            assert!(
                session_png(case, threshold) == reference_png(case, threshold),
                "export differs at threshold {threshold}"
            );
        }
    }
}

/// The deterministic 1200 × 1200 source of `reference_fixtures.rs`, whose export spans several
/// chunks.
const LARGE_SIDE: u32 = 1200;

fn large_rgba() -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..LARGE_SIDE {
        for x in 0..LARGE_SIDE {
            let red = (x * 131 + y * 137) ^ (x * y * 7);
            let green = (x * x + y * 3) ^ (y * y * 5);
            let blue = (x ^ y) * 29 + (x * 11 + y * 13) % 61;
            for channel in [red % 256, green % 256, blue % 256, 255] {
                pixels.push(u8::try_from(channel).unwrap());
            }
        }
    }
    pixels
}

#[test]
fn the_large_reference_export_matches_through_the_shared_session() {
    let large = |sampling| Case {
        width: LARGE_SIDE,
        height: LARGE_SIDE,
        cell: 5,
        font: 10,
        sampling,
    };
    for sampling in [Sampling::Classic, Sampling::Average] {
        let case = large(sampling);
        let source = DecodedSource::from_rgba(
            Dimensions::new(case.width, case.height).unwrap(),
            large_rgba(),
            DecodeLimits::default(),
        )
        .unwrap();
        let renderer = Renderer::new().unwrap();
        let mut sampler = Sampler::new(Arc::new(source));
        let grid = match sampling {
            Sampling::Classic => sampler.sample(case.cell, Sampling::Classic).unwrap(),
            Sampling::Average => {
                let rows = renderer
                    .aspect_geometry(case.cell, case.font)
                    .unwrap()
                    .source_cell_height;
                sampler
                    .sample_average_rect(case.cell, u32::from(rows))
                    .unwrap()
            }
        };
        let mut task = ArtworkPngTask::new(
            grid,
            Renderer::new().unwrap(),
            options(&case, 128),
            Generation::new(0),
            EXPORT_LIMITS,
        )
        .unwrap();
        task.advance(Generation::new(0), u64::MAX).unwrap();
        let mut expected = Vec::new();
        for chunk in task.drain_chunks().unwrap() {
            expected.extend(chunk);
        }
        for chunk in task.finish(Generation::new(0)).unwrap() {
            expected.extend(chunk);
        }
        let settings = PreviewSettings::new(case.cell, case.font, 128, sampling).unwrap();
        let mut session = Session::new(
            DecodedSource::from_rgba(
                Dimensions::new(case.width, case.height).unwrap(),
                large_rgba(),
                DecodeLimits::default(),
            )
            .unwrap(),
        )
        .unwrap();
        session.start_export(Generation::new(0), settings).unwrap();
        let mut actual = Vec::new();
        while session.stage().unwrap() != Stage::Ready {
            session
                .advance(Generation::new(0), DEFAULT_PIXEL_BUDGET)
                .unwrap();
            for chunk in session.drain_export(Generation::new(0)).unwrap() {
                actual.extend(chunk);
            }
        }
        for chunk in session.finish_export(Generation::new(0)).unwrap() {
            actual.extend(chunk);
        }
        assert!(actual == expected, "large {sampling:?} export differs");
    }
}

#[test]
fn the_classic_checker_golden_holds_through_the_shared_session() {
    let mut pixels = Vec::with_capacity(15 * 15 * 4);
    for y in 0..15_u32 {
        for x in 0..15_u32 {
            let value = if (x / 5 + y / 5) % 2 == 0 { 0 } else { 255 };
            pixels.extend_from_slice(&[value, value, value, 255]);
        }
    }
    let source = DecodedSource::from_rgba(
        Dimensions::new(15, 15).unwrap(),
        pixels,
        DecodeLimits::default(),
    )
    .unwrap();
    let mut session = Session::new(source).unwrap();
    let settings = PreviewSettings::new(5, 30, 128, Sampling::Classic).unwrap();
    session.start_preview(Generation::new(0), settings).unwrap();
    while session.stage().unwrap() != Stage::Ready {
        session
            .advance(Generation::new(0), DEFAULT_PIXEL_BUDGET)
            .unwrap();
    }
    let mut frame = session.finish(Generation::new(0)).unwrap();
    assert_eq!((frame.width(), frame.height()), (15, 15));
    assert_eq!(fnv64(&frame.take_pixels().unwrap()), CLASSIC_GOLDEN);
}
