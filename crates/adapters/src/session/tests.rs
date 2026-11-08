use std::fmt::Debug;
use std::sync::Arc;

use image_to_binary_core::{Dimensions, Generation, Sampling};
use image_to_binary_processing::{
    DecodeLimits, DecodedSource, RenderOptions, RenderedFrame, Renderer, Sampler,
};

use super::plan::reserve_zeroed;
use super::*;
use crate::ArtworkPngTask;

const MAX_SAFE: u64 = 9_007_199_254_740_991;

fn gradient(width: u32, height: u32) -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            for channel in [x * 9 + y * 3, x * 2 + y * 13, x * 7 + y * 5, 255] {
                pixels.push(u8::try_from(channel % 256).unwrap());
            }
        }
    }
    pixels
}

fn decoded(width: u32, height: u32) -> DecodedSource {
    DecodedSource::from_rgba(
        Dimensions::new(width, height).unwrap(),
        gradient(width, height),
        DecodeLimits::default(),
    )
    .unwrap()
}

fn new_session(width: u32, height: u32) -> Session {
    Session::new(decoded(width, height)).unwrap()
}

fn code<T: Debug>(result: Result<T, SessionError>) -> ErrorCode {
    result.unwrap_err().code()
}

fn g(value: u64) -> Generation {
    Generation::new(value)
}

fn settings(cell: u8, font: u8, threshold: u8, sampling: Sampling) -> PreviewSettings {
    PreviewSettings::new(cell, font, threshold, sampling).unwrap()
}

fn start(session: &mut Session, generation: u64, threshold: u8) -> Result<Stage, SessionError> {
    session.start_preview(g(generation), settings(5, 10, threshold, Sampling::Classic))
}

fn drive(session: &mut Session, generation: u64, budget: u64) -> Frame {
    while session.stage().unwrap() != Stage::Ready {
        session.advance(g(generation), budget).unwrap();
    }
    session.finish(g(generation)).unwrap()
}

fn reference(
    width: u32,
    height: u32,
    cell: u8,
    font: u8,
    threshold: u8,
    sampling: Sampling,
) -> RenderedFrame {
    let mut sampler = Sampler::new(Arc::new(decoded(width, height)));
    let mut renderer = Renderer::new().unwrap();
    let grid = match sampling {
        Sampling::Classic => sampler.sample(cell, sampling).unwrap(),
        Sampling::Average => {
            let rows = renderer
                .aspect_geometry(cell, font)
                .unwrap()
                .source_cell_height;
            sampler.sample_average_rect(cell, u32::from(rows)).unwrap()
        }
    };
    let options = RenderOptions {
        cell_size: cell,
        font_size: font,
        threshold,
        sampling,
        ..RenderOptions::default()
    };
    renderer.render_preview(&grid, options).unwrap()
}

#[test]
fn source_plan_validates_before_allocation() {
    assert_eq!(
        code(SourcePlan::new(0, 2, None)),
        ErrorCode::InvalidDimensions
    );
    assert_eq!(
        code(SourcePlan::new(4_294_967_296, 1, None)),
        ErrorCode::ResourceLimitExceeded
    );
    assert_eq!(
        code(SourcePlan::new(10_000, 5_001, None)),
        ErrorCode::ResourceLimitExceeded
    );
    assert_eq!(
        code(SourcePlan::new(2, 2, Some(3))),
        ErrorCode::ResourceLimitExceeded
    );
    assert_eq!(
        code(SourcePlan::new(2, 2, Some(0))),
        ErrorCode::InvalidSettings
    );
    assert_eq!(
        code(SourcePlan::new(2, 2, Some(50_000_001))),
        ErrorCode::InvalidSettings
    );
    let plan = SourcePlan::new(2, 2, Some(4)).unwrap();
    assert_eq!(code(plan.check_len(15)), ErrorCode::InvalidDimensions);
    plan.check_len(16).unwrap();
    assert_eq!(plan.allocate().unwrap().len(), 16);
}

#[test]
fn chunked_sessions_match_the_direct_preview() {
    for (sampling, cell, font) in [(Sampling::Classic, 7_u8, 14_u8), (Sampling::Average, 6, 18)] {
        let expected = reference(41, 29, cell, font, 128, sampling);
        for budget in [1, 17, 4096, MAX_SAFE] {
            let mut session = new_session(41, 29);
            session
                .start_preview(g(0), settings(cell, font, 128, sampling))
                .unwrap();
            let mut frame = drive(&mut session, 0, budget);
            assert_eq!(
                (frame.width(), frame.height()),
                (expected.width(), expected.height())
            );
            assert_eq!(frame.take_pixels().unwrap(), expected.pixels());
        }
    }
}

#[test]
fn stage_crossing_spends_no_budget_in_the_next_stage() {
    let mut session = new_session(30, 30);
    assert_eq!(start(&mut session, 0, 128).unwrap(), Stage::Sampling);
    assert_eq!(session.advance(g(0), 1_000_000).unwrap(), Stage::Preview);
    assert_eq!(session.progress().unwrap().0, 0);
    assert_eq!(session.advance(g(0), 1_000_000_000).unwrap(), Stage::Ready);
    let (done, total) = session.progress().unwrap();
    assert_eq!(done, total);
    assert_eq!(session.advance(g(0), 1).unwrap(), Stage::Ready);
}

#[test]
fn threshold_changes_reuse_the_grid_and_glyph_cache() {
    let mut session = new_session(41, 29);
    assert_eq!(start(&mut session, 0, 64).unwrap(), Stage::Sampling);
    let mut low = drive(&mut session, 0, MAX_SAFE);
    assert_eq!(start(&mut session, 1, 192).unwrap(), Stage::Preview);
    let mut high = drive(&mut session, 1, MAX_SAFE);
    assert_ne!(low.take_pixels().unwrap(), high.take_pixels().unwrap());
    assert!(
        session
            .live
            .as_ref()
            .unwrap()
            .renderer
            .cache_stats()
            .entries()
            > 0
    );
    let changed = session.start_preview(g(2), settings(6, 10, 64, Sampling::Classic));
    assert_eq!(changed.unwrap(), Stage::Sampling);
    assert_eq!(start(&mut session, 3, 64).unwrap(), Stage::Preview);
}

fn styled_settings(style: image_to_binary_processing::RenderStyle) -> PreviewSettings {
    settings(5, 10, 128, Sampling::Classic)
        .with_style(style)
        .unwrap()
}

fn tone_style() -> image_to_binary_processing::RenderStyle {
    image_to_binary_processing::RenderStyle {
        foreground: image_to_binary_processing::Rgb {
            red: 255,
            green: 200,
            blue: 0,
        },
        background: image_to_binary_processing::Rgb {
            red: 10,
            green: 0,
            blue: 40,
        },
        color: image_to_binary_processing::ColorMode::Source,
        contrast: 40,
        invert: true,
    }
}

#[test]
fn style_changes_reuse_the_sampled_grid_and_match_the_direct_renderer() {
    let mut session = new_session(41, 29);
    assert_eq!(start(&mut session, 0, 128).unwrap(), Stage::Sampling);
    let _plain = drive(&mut session, 0, MAX_SAFE);
    let stats = |session: &Session| {
        session.live.as_ref().unwrap().frames[0]
            .sampler
            .cache_stats()
    };
    let (computed, entries) = (stats(&session).computed(), stats(&session).entries());
    assert_eq!(computed, 1);

    let style = tone_style();
    assert_eq!(
        session.start_preview(g(1), styled_settings(style)).unwrap(),
        Stage::Preview
    );
    let mut styled = drive(&mut session, 1, MAX_SAFE);
    assert_eq!(stats(&session).computed(), computed);
    assert_eq!(stats(&session).entries(), entries);

    let mut sampler_direct = Sampler::new(Arc::new(decoded(41, 29)));
    let mut renderer = Renderer::new().unwrap();
    let grid = sampler_direct.sample(5, Sampling::Classic).unwrap();
    let options = RenderOptions {
        cell_size: 5,
        font_size: 10,
        threshold: 128,
        sampling: Sampling::Classic,
        style,
        ..RenderOptions::default()
    };
    let expected = renderer.render_preview(&grid, options).unwrap();
    assert_eq!(styled.take_pixels().unwrap(), expected.pixels());
}

/// Sampler and renderer counters: grids computed, grids retained, glyph masks rasterized and
/// glyph masks retained.
fn cache_counts(session: &Session) -> (u64, usize, u64, usize) {
    let live = session.live.as_ref().unwrap();
    let sampler = live.frames[0].sampler.cache_stats();
    let renderer = live.renderer.cache_stats();
    (
        sampler.computed(),
        sampler.entries(),
        renderer.rasterized(),
        renderer.entries(),
    )
}

#[test]
fn style_changes_reuse_the_grid_and_glyph_masks_for_both_samplings() {
    for sampling in [Sampling::Classic, Sampling::Average] {
        let mut session = new_session(41, 29);
        session
            .start_preview(g(0), settings(5, 10, 128, sampling))
            .unwrap();
        drive(&mut session, 0, MAX_SAFE);
        let before = cache_counts(&session);
        assert_eq!(before.0, 1, "{sampling:?}: one grid is sampled");
        assert!(
            before.2 > 0 && before.3 > 0,
            "{sampling:?}: glyph masks exist"
        );

        let restyled = settings(5, 10, 128, sampling)
            .with_style(tone_style())
            .unwrap();
        assert_eq!(
            session.start_preview(g(1), restyled).unwrap(),
            Stage::Preview,
            "{sampling:?}: a style-only change is a grid hit"
        );
        drive(&mut session, 1, MAX_SAFE);
        assert_eq!(
            cache_counts(&session),
            before,
            "{sampling:?}: no grid is sampled and no glyph mask is rasterized again"
        );
    }
}

#[test]
fn the_default_style_keeps_the_classic_pixels() {
    let mut session = new_session(41, 29);
    session
        .start_preview(g(0), settings(7, 14, 128, Sampling::Classic))
        .unwrap();
    let mut frame = drive(&mut session, 0, MAX_SAFE);
    let expected = reference(41, 29, 7, 14, 128, Sampling::Classic);
    assert_eq!(frame.take_pixels().unwrap(), expected.pixels());
}

#[test]
fn an_invalid_style_is_refused_before_it_reaches_a_job() {
    let session = new_session(41, 29);
    let bad = image_to_binary_processing::RenderStyle {
        contrast: 101,
        ..image_to_binary_processing::RenderStyle::default()
    };
    assert_eq!(
        settings(5, 10, 128, Sampling::Classic)
            .with_style(bad)
            .unwrap_err()
            .code(),
        ErrorCode::InvalidSettings
    );
    assert_eq!(session.stage().unwrap(), Stage::Idle);
}

#[test]
fn cancel_is_terminal_and_a_newer_job_recovers() {
    let expected = reference(30, 30, 5, 10, 128, Sampling::Classic);
    for steps in 0..4 {
        let mut session = new_session(30, 30);
        start(&mut session, 0, 128).unwrap();
        if steps == 1 {
            session.advance(g(0), 2).unwrap();
        } else if steps == 2 {
            session.advance(g(0), MAX_SAFE).unwrap();
            session.advance(g(0), 3).unwrap();
        } else if steps == 3 {
            while session.stage().unwrap() != Stage::Ready {
                session.advance(g(0), MAX_SAFE).unwrap();
            }
        }
        session.cancel(g(0)).unwrap();
        assert_eq!(session.stage().unwrap(), Stage::Failed);
        assert_eq!(code(session.advance(g(0), 1)), ErrorCode::Cancelled);
        assert_eq!(code(session.finish(g(0))), ErrorCode::Cancelled);
        session.cancel(g(0)).unwrap();
        assert_eq!(
            code(start(&mut session, 0, 128)),
            ErrorCode::StaleGeneration
        );
        start(&mut session, 1, 128).unwrap();
        let mut frame = drive(&mut session, 1, MAX_SAFE);
        assert_eq!(frame.take_pixels().unwrap(), expected.pixels());
    }
}

#[test]
fn cancelled_sampling_leaves_no_partial_cache() {
    let mut session = new_session(41, 29);
    start(&mut session, 0, 128).unwrap();
    session.advance(g(0), 1).unwrap();
    session.cancel(g(0)).unwrap();
    assert_eq!(start(&mut session, 1, 128).unwrap(), Stage::Sampling);
}

#[test]
fn stale_calls_never_disturb_the_current_job() {
    let mut session = new_session(30, 30);
    assert_eq!(code(session.advance(g(0), 1)), ErrorCode::NoJob);
    assert_eq!(code(session.cancel(g(0))), ErrorCode::NoJob);
    start(&mut session, 5, 128).unwrap();
    session.advance(g(5), 3).unwrap();
    for generation in [4, 5] {
        assert_eq!(
            code(start(&mut session, generation, 128)),
            ErrorCode::StaleGeneration
        );
    }
    for generation in [4, 6] {
        assert_eq!(
            code(session.advance(g(generation), 1)),
            ErrorCode::StaleGeneration
        );
        assert_eq!(
            code(session.finish(g(generation))),
            ErrorCode::StaleGeneration
        );
        assert_eq!(
            code(session.cancel(g(generation))),
            ErrorCode::StaleGeneration
        );
    }
    assert_eq!(session.progress().unwrap().0, 3);
    start(&mut session, 7, 128).unwrap();
    assert_eq!(code(session.advance(g(5), 1)), ErrorCode::StaleGeneration);
    drive(&mut session, 7, MAX_SAFE);
}

#[test]
fn usage_errors_and_disposal_are_predictable() {
    let mut session = new_session(30, 30);
    assert_eq!(session.stage().unwrap(), Stage::Idle);
    start(&mut session, 0, 128).unwrap();
    assert_eq!(code(session.advance(g(0), 0)), ErrorCode::InvalidArgument);
    assert_eq!(code(session.finish(g(0))), ErrorCode::JobIncomplete);
    let mut frame = drive(&mut session, 0, MAX_SAFE);
    assert_eq!(code(session.finish(g(0))), ErrorCode::NoJob);
    let pixels = frame.take_pixels().unwrap();
    assert_eq!(
        pixels.len(),
        usize::try_from(frame.width() * frame.height() * 4).unwrap()
    );
    assert_eq!(code(frame.take_pixels()), ErrorCode::FrameConsumed);
    session.dispose();
    session.dispose();
    assert_eq!(code(start(&mut session, 1, 128)), ErrorCode::Disposed);
    assert_eq!(code(session.advance(g(0), 1)), ErrorCode::Disposed);
    assert_eq!(code(session.finish(g(0))), ErrorCode::Disposed);
    assert_eq!(code(session.cancel(g(0))), ErrorCode::Disposed);
    assert_eq!(code(session.stage()), ErrorCode::Disposed);
    assert_eq!(code(session.progress()), ErrorCode::Disposed);
    assert_eq!(code(session.ensure_live()), ErrorCode::Disposed);
    assert!(session.live.is_none());
}

#[test]
fn failed_starts_keep_the_current_job_and_the_proposed_generation() {
    let mut session = new_session(41, 29);
    start(&mut session, 5, 128).unwrap();
    session.advance(g(5), 3).unwrap();
    let invalid = PreviewSettings {
        cell_size: 4,
        font_size: 10,
        threshold: 128,
        sampling: Sampling::Classic,
        mode: image_to_binary_core::RenderMode::Binary,
        style: image_to_binary_processing::RenderStyle::default(),
    };
    let failed =
        session
            .live_mut()
            .unwrap()
            .start(g(6), 0, invalid, JobKind::Preview, OutputKind::Png);
    assert_eq!(code(failed), ErrorCode::InvalidSettings);
    assert_eq!(session.live.as_ref().unwrap().newest, Some(5));
    assert_eq!(session.stage().unwrap(), Stage::Sampling);
    assert_eq!(session.progress().unwrap().0, 3);
    assert_eq!(session.advance(g(5), 2).unwrap(), Stage::Sampling);
    assert_eq!(session.progress().unwrap().0, 5);
    assert_eq!(start(&mut session, 6, 128).unwrap(), Stage::Sampling);
    assert_eq!(code(session.advance(g(5), 1)), ErrorCode::StaleGeneration);
    let mut frame = drive(&mut session, 6, MAX_SAFE);
    let expected = reference(41, 29, 5, 10, 128, Sampling::Classic);
    assert_eq!(frame.take_pixels().unwrap(), expected.pixels());
}

fn bmp_1x2() -> Vec<u8> {
    let mut bytes = b"BM".to_vec();
    for value in [62_u32, 0, 54, 40] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&1_i32.to_le_bytes());
    bytes.extend_from_slice(&2_i32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&24_u16.to_le_bytes());
    for value in [0_u32, 8, 2835, 2835, 0, 0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&[30, 20, 10, 0, 60, 50, 40, 0]);
    bytes
}

#[test]
fn encoded_plan_checks_the_ceiling_before_allocating() {
    let ceiling = usize::try_from(MAX_ENCODED_BYTES).unwrap();
    assert_eq!(
        code(EncodedPlan::new(ceiling + 1)),
        ErrorCode::EncodedTooLarge
    );
    assert_eq!(
        code(EncodedPlan::new(usize::MAX)),
        ErrorCode::EncodedTooLarge
    );
    assert_eq!(EncodedPlan::new(ceiling).unwrap().len, ceiling);
    assert_eq!(EncodedPlan::new(3).unwrap().allocate().unwrap(), vec![0; 3]);
    assert_eq!(
        code(reserve_zeroed(usize::MAX)),
        ErrorCode::ReservationFailed
    );
}

#[test]
fn encoded_sessions_expose_normalized_metadata_until_disposed() {
    let mut session = EncodedPlan::decode_session(bmp_1x2()).unwrap();
    let metadata = session.source_metadata().unwrap();
    assert_eq!(metadata.dimensions(), Dimensions::new(1, 2).unwrap());
    assert_eq!(metadata.channels(), 4);
    session.dispose();
    assert_eq!(code(session.source_metadata()), ErrorCode::Disposed);
    assert_eq!(code(session.source_metadata()), ErrorCode::Disposed);
    let rgba = new_session(7, 3).source_metadata().unwrap();
    assert_eq!(rgba.dimensions(), Dimensions::new(7, 3).unwrap());
    assert_eq!(rgba.channels(), 4);
}

#[test]
fn encoded_failures_use_stable_codes_and_build_no_session() {
    assert_eq!(
        code(EncodedPlan::decode_session(b"%PDF-1.7".to_vec())),
        ErrorCode::UnsupportedFormat
    );
    let mut truncated = bmp_1x2();
    truncated.truncate(30);
    assert_eq!(
        code(EncodedPlan::decode_session(truncated)),
        ErrorCode::DecodeFailed
    );
}

#[test]
fn original_sessions_return_exact_source_pixels_for_every_budget() {
    for budget in [1, 17, 4096, MAX_SAFE] {
        let mut session = new_session(41, 29);
        assert_eq!(
            session.start_original_preview(g(0)).unwrap(),
            Stage::Preview
        );
        assert_eq!(session.progress().unwrap(), (0, 41 * 29));
        let mut frame = drive(&mut session, 0, budget);
        assert_eq!((frame.width(), frame.height()), (41, 29));
        assert_eq!(frame.take_pixels().unwrap(), gradient(41, 29));
        assert_eq!(code(session.finish(g(0))), ErrorCode::NoJob);
    }
    let mut wide = new_session(1700, 3);
    wide.start_original_preview(g(0)).unwrap();
    let frame = drive(&mut wide, 0, MAX_SAFE);
    assert_eq!((frame.width(), frame.height()), (1600, 2));
}

#[test]
fn generations_are_monotonic_across_artwork_and_original_jobs() {
    let mut session = new_session(30, 30);
    session.start_original_preview(g(5)).unwrap();
    session.advance(g(5), 3).unwrap();
    for generation in [4, 5] {
        assert_eq!(
            code(session.start_original_preview(g(generation))),
            ErrorCode::StaleGeneration
        );
        assert_eq!(
            code(start(&mut session, generation, 128)),
            ErrorCode::StaleGeneration
        );
    }
    assert_eq!(session.stage().unwrap(), Stage::Preview);
    assert_eq!(session.progress().unwrap().0, 3);
    assert_eq!(start(&mut session, 6, 128).unwrap(), Stage::Sampling);
    assert_eq!(code(session.advance(g(5), 1)), ErrorCode::StaleGeneration);
    assert_eq!(
        session.start_original_preview(g(7)).unwrap(),
        Stage::Preview
    );
    assert_eq!(
        code(start(&mut session, 7, 128)),
        ErrorCode::StaleGeneration
    );
    assert_eq!(code(session.advance(g(6), 1)), ErrorCode::StaleGeneration);
    let mut frame = drive(&mut session, 7, MAX_SAFE);
    assert_eq!(frame.take_pixels().unwrap(), gradient(30, 30));
}

#[test]
fn invalid_starts_keep_an_accepted_original_job() {
    let mut session = new_session(41, 29);
    session.start_original_preview(g(5)).unwrap();
    session.advance(g(5), 3).unwrap();
    let invalid = PreviewSettings {
        cell_size: 4,
        font_size: 10,
        threshold: 128,
        sampling: Sampling::Classic,
        mode: image_to_binary_core::RenderMode::Binary,
        style: image_to_binary_processing::RenderStyle::default(),
    };
    let failed =
        session
            .live_mut()
            .unwrap()
            .start(g(6), 0, invalid, JobKind::Preview, OutputKind::Png);
    assert_eq!(code(failed), ErrorCode::InvalidSettings);
    assert_eq!(session.live.as_ref().unwrap().newest, Some(5));
    assert_eq!(session.stage().unwrap(), Stage::Preview);
    assert_eq!(session.progress().unwrap().0, 3);
    assert_eq!(session.advance(g(5), 2).unwrap(), Stage::Preview);
    assert_eq!(session.progress().unwrap().0, 5);
    let mut frame = drive(&mut session, 5, MAX_SAFE);
    assert_eq!(frame.take_pixels().unwrap(), gradient(41, 29));
}

#[test]
fn original_jobs_share_the_source_and_release_it_on_cancel_and_dispose() {
    let mut session = new_session(41, 29);
    session.start_original_preview(g(0)).unwrap();
    assert_eq!(
        Arc::strong_count(&session.live.as_ref().unwrap().frames[0].source),
        3
    );
    assert_eq!(code(session.finish(g(0))), ErrorCode::JobIncomplete);
    session.advance(g(0), 2).unwrap();
    session.cancel(g(0)).unwrap();
    assert_eq!(
        Arc::strong_count(&session.live.as_ref().unwrap().frames[0].source),
        2
    );
    assert_eq!(session.stage().unwrap(), Stage::Failed);
    assert_eq!(code(session.advance(g(0), 1)), ErrorCode::Cancelled);
    assert_eq!(code(session.finish(g(0))), ErrorCode::Cancelled);
    assert_eq!(
        code(session.start_original_preview(g(0))),
        ErrorCode::StaleGeneration
    );
    assert_eq!(
        session.start_original_preview(g(1)).unwrap(),
        Stage::Preview
    );
    session.dispose();
    assert_eq!(
        code(session.start_original_preview(g(2))),
        ErrorCode::Disposed
    );
    assert!(session.live.is_none());
}

const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
const IEND_CHUNK: [u8; 12] = [0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82];

fn native_png(
    width: u32,
    height: u32,
    cell: u8,
    font: u8,
    threshold: u8,
    sampling: Sampling,
) -> Vec<u8> {
    let mut sampler = Sampler::new(Arc::new(decoded(width, height)));
    let grid = match sampling {
        Sampling::Classic => sampler.sample(cell, sampling).unwrap(),
        Sampling::Average => {
            let rows = Renderer::new()
                .unwrap()
                .aspect_geometry(cell, font)
                .unwrap()
                .source_cell_height;
            sampler.sample_average_rect(cell, u32::from(rows)).unwrap()
        }
    };
    let options = RenderOptions {
        cell_size: cell,
        font_size: font,
        threshold,
        sampling,
        ..RenderOptions::default()
    };
    let mut task = ArtworkPngTask::new(
        grid,
        Renderer::new().unwrap(),
        options,
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

fn start_export(
    session: &mut Session,
    generation: u64,
    sampling: Sampling,
) -> Result<Stage, SessionError> {
    session.start_export(g(generation), settings(5, 10, 128, sampling))
}

/// Runs an export to completion, collecting drained chunks along the way.
fn export_bytes(session: &mut Session, generation: u64, budget: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    while session.stage().unwrap() != Stage::Ready {
        session.advance(g(generation), budget).unwrap();
        for chunk in session.drain_export(g(generation)).unwrap() {
            bytes.extend(chunk);
        }
    }
    for chunk in session.finish_export(g(generation)).unwrap() {
        bytes.extend(chunk);
    }
    bytes
}

fn encoding_row_pixels(session: &Session) -> u64 {
    match &session.live.as_ref().unwrap().job.as_ref().unwrap().phase {
        Phase::Encoding(task) => match task.as_ref() {
            ExportTask::Raster(raster) => raster.grid_row_units(),
            ExportTask::Gif(_) => panic!("expected a raster export"),
        },
        other => panic!("expected an encoding export, found {other:?}"),
    }
}

/// Samples an export until it reaches `Encoding` and returns one row's pixels.
fn row_pixels_after_sampling(session: &mut Session, cell: u8, font: u8, sampling: Sampling) -> u64 {
    session
        .start_export(g(0), settings(cell, font, 128, sampling))
        .unwrap();
    while session.stage().unwrap() == Stage::Sampling {
        session.advance(g(0), MAX_SAFE).unwrap();
    }
    encoding_row_pixels(session)
}

#[test]
fn exports_match_the_native_png_for_every_budget() {
    for (sampling, cell, font) in [(Sampling::Classic, 7_u8, 14_u8), (Sampling::Average, 6, 18)] {
        let expected = native_png(41, 29, cell, font, 128, sampling);
        assert_eq!(&expected[..8], &PNG_SIGNATURE);
        assert_eq!(&expected[expected.len() - 12..], &IEND_CHUNK);
        let row = row_pixels_after_sampling(&mut new_session(41, 29), cell, font, sampling);
        let budgets = [
            1,
            row.saturating_sub(1).max(1),
            row,
            3 * row + 1,
            17,
            4096,
            MAX_SAFE,
        ];
        for budget in budgets {
            let mut session = new_session(41, 29);
            session
                .start_export(g(0), settings(cell, font, 128, sampling))
                .unwrap();
            assert_eq!(export_bytes(&mut session, 0, budget), expected);
        }
    }
}

#[test]
fn export_budgets_convert_pixels_into_whole_grid_rows() {
    let mut session = new_session(30, 30);
    let row = row_pixels_after_sampling(&mut session, 5, 10, Sampling::Classic);
    assert_eq!(session.progress().unwrap(), (0, 6));
    assert_eq!(session.advance(g(0), 1).unwrap(), Stage::Encoding);
    assert_eq!(session.progress().unwrap().0, 1);
    assert_eq!(session.advance(g(0), 4 * row - 1).unwrap(), Stage::Encoding);
    assert_eq!(session.progress().unwrap().0, 4);
    assert_eq!(session.advance(g(0), 2 * row).unwrap(), Stage::Ready);
    assert_eq!(session.progress().unwrap(), (6, 6));
}

#[test]
fn mid_sized_budgets_leave_a_cancellation_boundary_before_publishing() {
    let expected = native_png(300, 200, 5, 10, 128, Sampling::Classic);
    let mut session = new_session(300, 200);
    let row = row_pixels_after_sampling(&mut session, 5, 10, Sampling::Classic);
    let budget = 4 * row;
    assert_eq!(session.advance(g(0), budget).unwrap(), Stage::Encoding);
    assert_eq!(session.progress().unwrap().0, 4);
    session.cancel(g(0)).unwrap();
    assert_eq!(code(session.advance(g(0), budget)), ErrorCode::Cancelled);
    assert_eq!(code(session.drain_export(g(0))), ErrorCode::Cancelled);
    assert_eq!(code(session.finish_export(g(0))), ErrorCode::Cancelled);

    let mut fresh = new_session(300, 200);
    start_export(&mut fresh, 1, Sampling::Classic).unwrap();
    let mut advances = 0_u32;
    while fresh.stage().unwrap() != Stage::Ready {
        fresh.advance(g(1), budget).unwrap();
        advances += 1;
    }
    assert!(
        advances > 1,
        "a mid-sized budget must need several advances"
    );
    let mut restarted = new_session(300, 200);
    start_export(&mut restarted, 1, Sampling::Classic).unwrap();
    assert_eq!(export_bytes(&mut restarted, 1, budget), expected);
}

#[test]
fn export_stages_run_sampling_then_encoding_then_ready() {
    let mut session = new_session(30, 30);
    assert_eq!(
        start_export(&mut session, 0, Sampling::Classic).unwrap(),
        Stage::Sampling
    );
    while session.stage().unwrap() == Stage::Sampling {
        session.advance(g(0), MAX_SAFE).unwrap();
    }
    assert_eq!(session.stage().unwrap(), Stage::Encoding);
    assert_eq!(session.progress().unwrap().0, 0);
    assert_eq!(session.advance(g(0), 2).unwrap(), Stage::Encoding);
    assert_eq!(session.progress().unwrap().0, 1);
    assert_eq!(session.advance(g(0), MAX_SAFE).unwrap(), Stage::Ready);
    let (done, total) = session.progress().unwrap();
    assert_eq!(done, total);
    assert_eq!(session.advance(g(0), 1).unwrap(), Stage::Ready);
    assert!(!session.finish_export(g(0)).unwrap().is_empty());
    assert_eq!(session.stage().unwrap(), Stage::Idle);
}

#[test]
fn export_drains_are_in_order_and_exactly_once() {
    let expected = native_png(41, 29, 5, 10, 128, Sampling::Classic);
    let mut session = new_session(41, 29);
    start_export(&mut session, 0, Sampling::Classic).unwrap();
    let mut drained = Vec::new();
    while session.stage().unwrap() != Stage::Ready {
        session.advance(g(0), 3).unwrap();
        drained.extend(session.drain_export(g(0)).unwrap());
        assert!(session.drain_export(g(0)).unwrap().is_empty());
    }
    let tail = session.finish_export(g(0)).unwrap();
    assert_eq!(code(session.finish_export(g(0))), ErrorCode::NoJob);
    let joined: Vec<u8> = drained.into_iter().chain(tail).flatten().collect();
    assert_eq!(joined, expected);
}

#[test]
fn export_sampling_reuses_the_preview_grid_cache() {
    let expected = native_png(41, 29, 5, 10, 128, Sampling::Classic);
    let mut session = new_session(41, 29);
    start(&mut session, 0, 128).unwrap();
    drive(&mut session, 0, MAX_SAFE);
    assert_eq!(
        start_export(&mut session, 1, Sampling::Classic).unwrap(),
        Stage::Encoding
    );
    assert_eq!(export_bytes(&mut session, 1, 9), expected);
}

#[test]
fn export_usage_errors_keep_the_job_and_use_stable_codes() {
    let mut session = new_session(30, 30);
    assert_eq!(code(session.drain_export(g(0))), ErrorCode::NoJob);
    assert_eq!(code(session.finish_export(g(0))), ErrorCode::NoJob);
    start_export(&mut session, 0, Sampling::Classic).unwrap();
    assert_eq!(code(session.advance(g(0), 0)), ErrorCode::InvalidArgument);
    assert_eq!(session.stage().unwrap(), Stage::Sampling);
    assert_eq!(code(session.finish_export(g(0))), ErrorCode::JobIncomplete);
    assert!(session.drain_export(g(0)).unwrap().is_empty());
    assert_eq!(code(session.finish(g(0))), ErrorCode::NoJob);
    assert_eq!(
        code(start_export(&mut session, 0, Sampling::Classic)),
        ErrorCode::StaleGeneration
    );
    assert_eq!(code(session.drain_export(g(1))), ErrorCode::StaleGeneration);
    assert_eq!(
        code(session.finish_export(g(1))),
        ErrorCode::StaleGeneration
    );
    assert_eq!(
        PreviewSettings::new(4, 10, 128, Sampling::Classic)
            .unwrap_err()
            .code(),
        ErrorCode::InvalidSettings
    );
    assert_eq!(session.stage().unwrap(), Stage::Sampling);
    while session.stage().unwrap() != Stage::Encoding {
        session.advance(g(0), MAX_SAFE).unwrap();
    }
    assert_eq!(code(session.finish_export(g(0))), ErrorCode::JobIncomplete);
    session.advance(g(0), 1).unwrap();
    assert_eq!(code(session.finish_export(g(0))), ErrorCode::JobIncomplete);
    assert_eq!(session.stage().unwrap(), Stage::Encoding);
    export_bytes(&mut session, 0, MAX_SAFE);
    assert_eq!(code(session.finish_export(g(0))), ErrorCode::NoJob);
}

#[test]
fn preview_and_export_calls_never_act_on_the_other_kind() {
    let mut session = new_session(30, 30);
    start_export(&mut session, 0, Sampling::Classic).unwrap();
    session.advance(g(0), MAX_SAFE).unwrap();
    assert_eq!(code(session.finish(g(0))), ErrorCode::NoJob);
    assert_eq!(session.stage().unwrap(), Stage::Encoding);
    session.advance(g(0), MAX_SAFE).unwrap();
    assert_eq!(code(session.finish(g(0))), ErrorCode::NoJob);
    assert_eq!(session.stage().unwrap(), Stage::Ready);
    assert!(!session.finish_export(g(0)).unwrap().is_empty());

    start(&mut session, 1, 128).unwrap();
    assert_eq!(code(session.finish_export(g(1))), ErrorCode::NoJob);
    assert_eq!(code(session.drain_export(g(1))), ErrorCode::NoJob);
    assert_eq!(session.stage().unwrap(), Stage::Preview);
    drive(&mut session, 1, MAX_SAFE);
    assert_eq!(code(session.drain_export(g(1))), ErrorCode::NoJob);
    assert_eq!(code(session.finish_export(g(1))), ErrorCode::NoJob);
}

#[test]
fn exports_and_previews_supersede_each_other_by_generation() {
    let expected = native_png(30, 30, 5, 10, 128, Sampling::Classic);
    let mut session = new_session(30, 30);
    start_export(&mut session, 0, Sampling::Classic).unwrap();
    session.advance(g(0), 3).unwrap();
    assert_eq!(start(&mut session, 1, 128).unwrap(), Stage::Sampling);
    assert_eq!(code(session.advance(g(0), 1)), ErrorCode::StaleGeneration);
    assert_eq!(code(session.drain_export(g(0))), ErrorCode::StaleGeneration);
    assert_eq!(
        code(session.finish_export(g(0))),
        ErrorCode::StaleGeneration
    );
    assert_eq!(code(session.cancel(g(0))), ErrorCode::StaleGeneration);
    drive(&mut session, 1, MAX_SAFE);

    start_export(&mut session, 2, Sampling::Classic).unwrap();
    assert_eq!(code(session.advance(g(1), 1)), ErrorCode::StaleGeneration);
    assert_eq!(code(session.finish(g(1))), ErrorCode::StaleGeneration);
    assert_eq!(export_bytes(&mut session, 2, MAX_SAFE), expected);
}

#[test]
fn cancelled_exports_are_terminal_and_a_newer_export_recovers() {
    let expected = native_png(30, 30, 5, 10, 128, Sampling::Classic);
    for steps in 0..3 {
        let mut session = new_session(30, 30);
        start_export(&mut session, 0, Sampling::Classic).unwrap();
        for _ in 0..steps {
            session.advance(g(0), MAX_SAFE).unwrap();
        }
        session.cancel(g(0)).unwrap();
        session.cancel(g(0)).unwrap();
        assert_eq!(session.stage().unwrap(), Stage::Failed);
        assert_eq!(code(session.advance(g(0), 1)), ErrorCode::Cancelled);
        assert_eq!(code(session.drain_export(g(0))), ErrorCode::Cancelled);
        assert_eq!(code(session.finish_export(g(0))), ErrorCode::Cancelled);
        assert_eq!(
            code(start_export(&mut session, 0, Sampling::Classic)),
            ErrorCode::StaleGeneration
        );
        start_export(&mut session, 1, Sampling::Classic).unwrap();
        assert_eq!(export_bytes(&mut session, 1, 5), expected);
    }
}

#[test]
fn disposal_mid_export_releases_the_job_and_every_call_is_disposed() {
    let mut session = new_session(30, 30);
    start_export(&mut session, 0, Sampling::Average).unwrap();
    session.advance(g(0), MAX_SAFE).unwrap();
    session.advance(g(0), 2).unwrap();
    session.dispose();
    session.dispose();
    assert!(session.live.is_none());
    assert_eq!(code(session.drain_export(g(0))), ErrorCode::Disposed);
    assert_eq!(code(session.finish_export(g(0))), ErrorCode::Disposed);
    assert_eq!(code(session.advance(g(0), 1)), ErrorCode::Disposed);
    assert_eq!(
        code(start_export(&mut session, 1, Sampling::Classic)),
        ErrorCode::Disposed
    );
    assert_eq!(code(session.stage()), ErrorCode::Disposed);
}

#[test]
fn wrong_kind_calls_report_no_job_even_when_the_job_failed() {
    let mut preview = new_session(30, 30);
    start(&mut preview, 0, 128).unwrap();
    preview.cancel(g(0)).unwrap();
    assert_eq!(code(preview.drain_export(g(0))), ErrorCode::NoJob);
    assert_eq!(code(preview.finish_export(g(0))), ErrorCode::NoJob);
    assert_eq!(code(preview.finish(g(0))), ErrorCode::Cancelled);
    assert_eq!(preview.stage().unwrap(), Stage::Failed);

    let mut export = new_session(30, 30);
    start_export(&mut export, 0, Sampling::Classic).unwrap();
    export.cancel(g(0)).unwrap();
    assert_eq!(code(export.finish(g(0))), ErrorCode::NoJob);
    assert_eq!(code(export.finish_export(g(0))), ErrorCode::Cancelled);
    assert_eq!(export.stage().unwrap(), Stage::Failed);
    assert_eq!(code(export.drain_export(g(0))), ErrorCode::Cancelled);
    assert_eq!(
        code(start_export(&mut export, 0, Sampling::Classic)),
        ErrorCode::StaleGeneration
    );
    assert_eq!(
        start_export(&mut export, 1, Sampling::Classic).unwrap(),
        Stage::Sampling
    );
}

#[test]
fn text_exports_follow_the_contract_for_every_budget() {
    let mut reference: Option<Vec<u8>> = None;
    for budget in [1, 7, 4_096, 1 << 20] {
        let mut session = new_session(37, 23);
        session
            .start_export_as(
                g(0),
                settings(5, 10, 128, Sampling::Classic),
                OutputKind::Text,
            )
            .unwrap();
        let bytes = export_bytes(&mut session, 0, budget);
        assert!(!bytes.starts_with(&[0xEF, 0xBB, 0xBF]), "no BOM");
        assert!(!bytes.contains(&b'\r'), "LF only");
        assert_eq!(bytes.last(), Some(&b'\n'), "trailing LF");
        let text = std::str::from_utf8(&bytes).unwrap();
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        let width = lines[0].len() - 1;
        assert!(width > 0 && !lines.is_empty());
        for line in &lines {
            assert!(line.ends_with('\n'));
            assert_eq!(line.len() - 1, width, "every line has the same width");
        }
        match &reference {
            None => reference = Some(bytes),
            Some(first) => assert_eq!(&bytes, first, "budget {budget} changed the bytes"),
        }
    }
}

#[test]
fn text_export_equals_the_direct_text_rows_for_budgets_one_seven_and_4096() {
    let options = RenderOptions {
        cell_size: 5,
        font_size: 10,
        threshold: 128,
        sampling: Sampling::Classic,
        ..RenderOptions::default()
    };
    let mut sampler = Sampler::new(Arc::new(decoded(37, 23)));
    let grid = sampler.sample(5, Sampling::Classic).unwrap();
    let direct = Renderer::new()
        .unwrap()
        .text_rows(&grid, options, 0, grid.height())
        .unwrap();
    for budget in [1, 7, 4_096] {
        let mut session = new_session(37, 23);
        session
            .start_export_as(
                g(0),
                settings(5, 10, 128, Sampling::Classic),
                OutputKind::Text,
            )
            .unwrap();
        assert_eq!(
            export_bytes(&mut session, 0, budget),
            direct,
            "budget {budget}"
        );
    }
}

#[test]
fn a_text_export_is_not_a_png_and_png_stays_the_default() {
    let mut session = new_session(16, 16);
    session
        .start_export(g(0), settings(5, 10, 128, Sampling::Classic))
        .unwrap();
    let png = export_bytes(&mut session, 0, 1 << 20);
    assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
    let mut session = new_session(16, 16);
    session
        .start_export_as(
            g(0),
            settings(5, 10, 128, Sampling::Classic),
            OutputKind::Text,
        )
        .unwrap();
    let text = export_bytes(&mut session, 0, 1 << 20);
    assert!(!text.starts_with(&[0x89, b'P', b'N', b'G']));
}
