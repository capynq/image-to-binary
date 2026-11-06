use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
use image_to_binary_core::{ContractError, Generation, Ramp, RenderMode, Sampling};
use image_to_binary_processing::{
    decode, ColorMode, DecodeLimits, RenderError, RenderOptions, RenderStyle, Renderer, Rgb,
    Sampler, MAX_RENDER_ROW_BYTES,
};
use std::io::Cursor;
use std::sync::Arc;

fn source(
    width: u32,
    height: u32,
    f: impl Fn(u32, u32) -> [u8; 4],
) -> Arc<image_to_binary_processing::DecodedSource> {
    let image = ImageBuffer::from_fn(width, height, |x, y| Rgba(f(x, y)));
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    Arc::new(decode(&bytes.into_inner(), DecodeLimits::default()).unwrap())
}

fn mixed_grid(width: u32, height: u32) -> Arc<image_to_binary_processing::SampleGrid> {
    let source = source(width * 5, height * 5, |x, y| {
        let dark = ((x / 5) + (y / 5)) % 2 == 0;
        if dark {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        }
    });
    Sampler::new(source).sample(5, Sampling::Classic).unwrap()
}

fn options() -> RenderOptions {
    RenderOptions {
        cell_size: 5,
        font_size: 30,
        ..RenderOptions::default()
    }
}

fn fnv64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn nearest_center(
    frame: &image_to_binary_processing::RenderedFrame,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let mut output = Vec::with_capacity((width as usize) * (height as usize) * 4);
    for y in 0..height {
        let source_y = usize::try_from(
            (u64::from(y) * 2 + 1) * u64::from(frame.height()) / (u64::from(height) * 2),
        )
        .unwrap();
        for x in 0..width {
            let source_x = usize::try_from(
                (u64::from(x) * 2 + 1) * u64::from(frame.width()) / (u64::from(width) * 2),
            )
            .unwrap();
            let index = (source_y * frame.width() as usize + source_x) * 4;
            output.extend_from_slice(&frame.pixels()[index..index + 4]);
        }
    }
    output
}

#[test]
fn classic_mixed_fixture_has_stable_golden_and_both_glyphs() {
    let grid = mixed_grid(3, 3);
    let mut renderer = Renderer::new().unwrap();
    let frame = renderer.render(&grid, options()).unwrap();
    assert_eq!(frame.width(), 15);
    assert_eq!(frame.height(), 15);
    assert_eq!(fnv64(frame.pixels()), 15_743_520_938_017_280_556);
    assert!(frame.pixels().chunks_exact(4).any(|p| p[1] > 0));
    assert!(frame.pixels().chunks_exact(4).any(|p| p == [0, 0, 0, 255]));
}

#[test]
fn rows_cover_the_complete_frame_at_large_font() {
    let grid = mixed_grid(3, 3);
    let mut renderer = Renderer::new().unwrap();
    let full = renderer.render(&grid, options()).unwrap();
    let mut rows = Vec::new();
    for start in 0..grid.height() {
        rows.extend(renderer.render_rows(&grid, options(), start, 1).unwrap());
    }
    assert_eq!(rows.len(), full.pixels().len());
    assert_eq!(rows, full.pixels());
}

#[test]
fn render_rows_rejects_zero_count_after_settings_and_geometry_checks() {
    let grid = mixed_grid(3, 3);
    let mut renderer = Renderer::new().unwrap();
    for start in [0, 3] {
        assert!(matches!(
            renderer.render_rows(&grid, options(), start, 0),
            Err(RenderError::Contract(ContractError::InvalidDimensions))
        ));
    }
    let wrong_width = RenderOptions {
        cell_size: 6,
        ..options()
    };
    assert!(matches!(
        renderer.render_rows(&grid, wrong_width, 0, 0),
        Err(RenderError::Contract(ContractError::InvalidSettings(_)))
    ));
}

#[test]
fn render_rows_enforces_configured_limit_at_exact_boundary() {
    let grid = mixed_grid(3, 3);
    let mut renderer = Renderer::new().unwrap();
    let row_bytes = 15 * 5 * 4;
    let exact = RenderOptions {
        max_output_bytes: row_bytes,
        ..options()
    };
    assert_eq!(
        renderer.render_rows(&grid, exact, 0, 1).unwrap().len(),
        usize::try_from(row_bytes).unwrap()
    );
    let below = RenderOptions {
        max_output_bytes: row_bytes - 1,
        ..options()
    };
    assert!(matches!(
        renderer.render_rows(&grid, below, 0, 1),
        Err(RenderError::Contract(ContractError::ResourceLimitExceeded))
    ));
    assert!(matches!(
        renderer.render_rows(&grid, exact, 0, 2),
        Err(RenderError::Contract(ContractError::ResourceLimitExceeded))
    ));
}

#[test]
fn render_rows_fixed_ceiling_rejects_oversized_batch_under_unbounded_option() {
    let mut renderer = Renderer::new().unwrap();
    let geometry = renderer.aspect_geometry(5, 30).unwrap();
    let grid_width = 1000_u32;
    let grid_height = 30_u32;
    let source = source(
        grid_width * u32::from(geometry.source_cell_width),
        grid_height * u32::from(geometry.source_cell_height),
        |_, _| [0, 0, 0, 255],
    );
    let grid = Sampler::new(source)
        .sample_average_rect(5, u32::from(geometry.source_cell_height))
        .unwrap();
    assert_eq!(grid.width(), grid_width);
    assert_eq!(grid.height(), grid_height);
    let unbounded = RenderOptions {
        cell_size: 5,
        font_size: 30,
        sampling: Sampling::Average,
        max_output_bytes: u64::MAX,
        ..RenderOptions::default()
    };
    let requested = u64::from(grid_width)
        * u64::from(geometry.output_cell_width)
        * u64::from(grid_height)
        * u64::from(geometry.output_cell_height)
        * 4;
    assert!(requested > MAX_RENDER_ROW_BYTES);
    assert!(matches!(
        renderer.render_rows(&grid, unbounded, 0, grid_height),
        Err(RenderError::Contract(ContractError::ResourceLimitExceeded))
    ));
}

#[test]
fn preview_same_geometry_is_full_frame_and_extreme_edges_are_bounded() {
    let grid = mixed_grid(3, 3);
    let mut renderer = Renderer::new().unwrap();
    let full = renderer.render(&grid, options()).unwrap();
    let preview = renderer.render_preview(&grid, options()).unwrap();
    assert_eq!(preview, full);

    let wide = mixed_grid(501, 1);
    let wide_full = renderer.render(&wide, options()).unwrap();
    let preview = renderer.render_preview(&wide, options()).unwrap();
    assert!(preview.width().max(preview.height()) <= 1600);
    assert_eq!(
        preview.pixels(),
        nearest_center(&wide_full, preview.width(), preview.height())
    );
    let tall = mixed_grid(1, 501);
    let tall_full = renderer.render(&tall, options()).unwrap();
    let preview = renderer.render_preview(&tall, options()).unwrap();
    assert!(preview.width().max(preview.height()) <= 1600);
    assert_eq!(
        preview.pixels(),
        nearest_center(&tall_full, preview.width(), preview.height())
    );
}

#[test]
fn render_entrypoints_reject_grid_geometry_mismatches() {
    let grid = mixed_grid(3, 3);
    let mut renderer = Renderer::new().unwrap();
    let wrong_width = RenderOptions {
        cell_size: 6,
        ..options()
    };
    assert!(renderer.render(&grid, wrong_width).is_err());
    assert!(renderer.render_preview(&grid, wrong_width).is_err());
    assert!(renderer.render_rows(&grid, wrong_width, 0, 1).is_err());

    let geometry = renderer.aspect_geometry(5, 30).unwrap();
    let rect = Sampler::new(source(
        15,
        u32::from(geometry.source_cell_height) * 3,
        |_, _| [0, 0, 0, 255],
    ))
    .sample_average_rect(5, u32::from(geometry.source_cell_height))
    .unwrap();
    let wrong_sampling = RenderOptions {
        sampling: Sampling::Classic,
        ..options()
    };
    assert!(renderer.render(&rect, wrong_sampling).is_err());
    assert!(renderer.render_preview(&rect, wrong_sampling).is_err());
    assert!(renderer.render_rows(&rect, wrong_sampling, 0, 1).is_err());

    let wrong_height = u32::from(geometry.source_cell_height) + 1;
    let wrong_rect = Sampler::new(source(15, wrong_height * 3, |_, _| [0, 0, 0, 255]))
        .sample_average_rect(5, wrong_height)
        .unwrap();
    let average = RenderOptions {
        sampling: Sampling::Average,
        ..options()
    };
    assert!(renderer.render(&wrong_rect, average).is_err());
    assert!(renderer.render_preview(&wrong_rect, average).is_err());
    assert!(renderer.render_rows(&wrong_rect, average, 0, 1).is_err());
}

#[test]
fn proper_average_geometry_is_consistent_across_full_preview_and_rows() {
    let renderer = Renderer::new().unwrap();
    let geometry = renderer.aspect_geometry(5, 30).unwrap();
    let source_height = u32::from(geometry.source_cell_height) * 3 + 2;
    let grid = Sampler::new(source(17, source_height, |x, y| {
        if ((x / 5) + (y / u32::from(geometry.source_cell_height))) % 2 == 0 {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        }
    }))
    .sample_average_rect(5, u32::from(geometry.source_cell_height))
    .unwrap();
    let options = RenderOptions {
        sampling: Sampling::Average,
        ..options()
    };
    let mut renderer = Renderer::new().unwrap();
    let full = renderer.render(&grid, options).unwrap();
    let preview = renderer.render_preview(&grid, options).unwrap();
    assert_eq!(preview, full);
    let mut rows = Vec::new();
    for start in 0..grid.height() {
        rows.extend(renderer.render_rows(&grid, options, start, 1).unwrap());
    }
    assert_eq!(rows, full.pixels());
}

#[test]
fn cache_zero_and_exact_single_mask_capacity_behave_as_bounded_cache() {
    let dark = mixed_grid(1, 1);
    let light = {
        let source = source(5, 5, |_, _| [255, 255, 255, 255]);
        Sampler::new(source).sample(5, Sampling::Classic).unwrap()
    };
    let mut uncached = Renderer::with_cache_capacity(0).unwrap();
    uncached.render(&dark, options()).unwrap();
    assert_eq!(uncached.cache_stats().entries(), 0);

    let mut probe = Renderer::with_cache_capacity(1024 * 1024).unwrap();
    probe.render(&dark, options()).unwrap();
    let one_mask_bytes = probe.cache_stats().bytes();
    assert!(one_mask_bytes > 0);
    let mut exact = Renderer::with_cache_capacity(one_mask_bytes).unwrap();
    exact.render(&dark, options()).unwrap();
    assert_eq!(exact.cache_stats().entries(), 1);
    assert_eq!(exact.cache_stats().bytes(), one_mask_bytes);
    exact.render(&light, options()).unwrap();
    assert_eq!(exact.cache_stats().entries(), 1);
    assert_eq!(exact.cache_stats().bytes(), one_mask_bytes);
}

#[test]
fn into_pixels_moves_the_frame_buffer_without_copying() {
    let grid = mixed_grid(3, 3);
    let mut renderer = Renderer::new().unwrap();
    let frame = renderer.render(&grid, options()).unwrap();
    let copy = frame.pixels().to_vec();
    let pointer = frame.pixels().as_ptr();
    let pixels = frame.into_pixels();
    assert_eq!(pixels, copy);
    assert_eq!(pixels.as_ptr(), pointer);
}

fn average_grid(width: u32, rows: u32) -> Arc<image_to_binary_processing::SampleGrid> {
    let renderer = Renderer::new().unwrap();
    let cell_height = u32::from(renderer.aspect_geometry(5, 30).unwrap().source_cell_height);
    let source = source(width * 5, cell_height * rows, |x, y| {
        if ((x / 5) + (y / cell_height)) % 2 == 0 {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        }
    });
    Sampler::new(source)
        .sample_average_rect(5, cell_height)
        .unwrap()
}

fn render_in_batches(
    renderer: &mut Renderer,
    grid: &image_to_binary_processing::SampleGrid,
    options: RenderOptions,
    batch: u32,
) -> Vec<u8> {
    let mut rows = Vec::new();
    let mut start = 0;
    while start < grid.height() {
        let count = batch.min(grid.height() - start);
        rows.extend(renderer.render_rows(grid, options, start, count).unwrap());
        start += count;
    }
    rows
}

#[test]
fn multi_row_batches_with_short_tail_match_full_frame_for_both_samplings() {
    let classic = mixed_grid(3, 6);
    let average = average_grid(3, 6);
    let average_options = RenderOptions {
        sampling: Sampling::Average,
        ..options()
    };
    let mut renderer = Renderer::new().unwrap();
    for (grid, render_options) in [(&classic, options()), (&average, average_options)] {
        assert_eq!(grid.height(), 6);
        let full = renderer.render(grid, render_options).unwrap();
        for batch in [4, 5] {
            let rows = render_in_batches(&mut renderer, grid, render_options, batch);
            assert_eq!(rows.len(), full.pixels().len());
            assert_eq!(rows, full.pixels());
        }
    }
}

#[test]
fn render_rows_rejects_ranges_past_grid_end_and_u32_overflow() {
    let grid = mixed_grid(3, 3);
    let mut renderer = Renderer::new().unwrap();
    for (start, count) in [(0, 4), (1, 3), (2, 2), (3, 1), (7, 1)] {
        assert!(
            matches!(
                renderer.render_rows(&grid, options(), start, count),
                Err(RenderError::Contract(ContractError::InvalidDimensions))
            ),
            "start {start} count {count} should exceed the grid"
        );
    }
    for (start, count) in [(u32::MAX, 1), (u32::MAX - 1, 2), (1, u32::MAX)] {
        assert!(
            matches!(
                renderer.render_rows(&grid, options(), start, count),
                Err(RenderError::Contract(ContractError::ArithmeticOverflow))
            ),
            "start {start} count {count} should overflow u32"
        );
    }
}

#[test]
fn row_batches_from_transparent_source_are_fully_opaque_and_match_full_frame() {
    let grid = Sampler::new(source(15, 15, |x, y| match ((x / 5) + (y / 5)) % 3 {
        0 => [0, 0, 0, 0],
        1 => [255, 255, 255, 128],
        _ => [0, 0, 0, 255],
    }))
    .sample(5, Sampling::Classic)
    .unwrap();
    let mut renderer = Renderer::new().unwrap();
    let full = renderer.render(&grid, options()).unwrap();
    assert!(full.pixels().chunks_exact(4).all(|pixel| pixel[3] == 255));
    let rows = render_in_batches(&mut renderer, &grid, options(), 2);
    assert_eq!(rows, full.pixels());
    assert!(rows.chunks_exact(4).all(|pixel| pixel[3] == 255));
}

fn mode_options(mode: RenderMode) -> RenderOptions {
    RenderOptions {
        cell_size: 5,
        font_size: 30,
        sampling: Sampling::Average,
        mode,
        ..RenderOptions::default()
    }
}

fn gray_cell_value(column: u32, row: u32) -> u8 {
    u8::try_from((column * 37 + row * 91 + 13) % 256).unwrap()
}

fn gradient_grid(
    columns: u32,
    rows: u32,
    partial: bool,
) -> Arc<image_to_binary_processing::SampleGrid> {
    let cell_height = u32::from(
        Renderer::new()
            .unwrap()
            .aspect_geometry(5, 30)
            .unwrap()
            .source_cell_height,
    );
    let (extra_width, extra_height) = if partial { (2, 3) } else { (0, 0) };
    let source = source(
        columns * 5 + extra_width,
        cell_height * rows + extra_height,
        |x, y| {
            let value = gray_cell_value(x / 5, y / cell_height);
            [value, value, value, 255]
        },
    );
    Sampler::new(source)
        .sample_average_rect(5, cell_height)
        .unwrap()
}

fn render_batches(
    renderer: &mut Renderer,
    grid: &image_to_binary_processing::SampleGrid,
    options: RenderOptions,
    batch: u32,
) -> Vec<u8> {
    let mut rows = Vec::new();
    let mut start = 0;
    while start < grid.height() {
        let count = batch.min(grid.height() - start);
        rows.extend(renderer.render_rows(grid, options, start, count).unwrap());
        start += count;
    }
    rows
}

const GOLDEN_ASCII_DEFAULT: u64 = 1_253_096_222_777_359_191;
const GOLDEN_ASCII_CUSTOM: u64 = 17_704_069_554_246_881_278;
const GOLDEN_BLOCK: u64 = 9_404_247_626_114_037_653;

#[test]
fn ascii_and_block_goldens_are_pinned_and_deterministic() {
    let grid = gradient_grid(3, 3, false);
    let custom = Ramp::new("-=+#").unwrap();
    let cases = [
        (RenderMode::Ascii(Ramp::DEFAULT_ASCII), GOLDEN_ASCII_DEFAULT),
        (RenderMode::Ascii(custom), GOLDEN_ASCII_CUSTOM),
        (RenderMode::Block, GOLDEN_BLOCK),
    ];
    for (mode, expected) in cases {
        let options = mode_options(mode);
        let mut renderer = Renderer::new().unwrap();
        let first = renderer.render(&grid, options).unwrap();
        let second = renderer.render(&grid, options).unwrap();
        assert_eq!(first, second);
        assert!(first.pixels().chunks_exact(4).all(|pixel| pixel[3] == 255));
        assert_eq!(fnv64(first.pixels()), expected, "{mode:?}");
    }
}

#[test]
fn every_mode_matches_concatenated_row_batches_including_short_tails() {
    let grid = gradient_grid(4, 6, true);
    assert_eq!(grid.height(), 7);
    let custom = Ramp::new(" .:-=+*#%@").unwrap();
    for mode in [
        RenderMode::Ascii(Ramp::DEFAULT_ASCII),
        RenderMode::Ascii(custom),
        RenderMode::Block,
    ] {
        let options = mode_options(mode);
        let mut renderer = Renderer::new().unwrap();
        let full = renderer.render(&grid, options).unwrap();
        for batch in [1, 3, 4, 5] {
            let rows = render_batches(&mut renderer, &grid, options, batch);
            assert_eq!(rows, full.pixels(), "{mode:?} batch {batch}");
        }
    }
}

#[test]
fn every_mode_preview_matches_export_geometry_with_partial_cells() {
    let grid = gradient_grid(4, 6, true);
    let mut renderer = Renderer::new().unwrap();
    for mode in [
        RenderMode::Binary,
        RenderMode::Ascii(Ramp::DEFAULT_ASCII),
        RenderMode::Block,
    ] {
        let options = mode_options(mode);
        let full = renderer.render(&grid, options).unwrap();
        assert_eq!(
            renderer.frame_dimensions(&grid, options).unwrap(),
            (full.width(), full.height())
        );
        assert_eq!(renderer.render_preview(&grid, options).unwrap(), full);
        let mut task = renderer
            .preview_task(Arc::clone(&grid), options, Generation::new(5))
            .unwrap();
        while task.progress().completed() < task.progress().total() {
            task.advance(7, Generation::new(5)).unwrap();
        }
        assert_eq!(task.finish(Generation::new(5)).unwrap(), full);
    }
}

#[test]
fn scaled_preview_of_ascii_is_identical_across_incremental_budgets() {
    let grid = gradient_grid(400, 2, true);
    let options = mode_options(RenderMode::Ascii(Ramp::DEFAULT_ASCII));
    let mut renderer = Renderer::new().unwrap();
    let expected = renderer.render_preview(&grid, options).unwrap();
    assert!(expected.width().max(expected.height()) <= 1600);
    for budget in [1, 4096, u64::MAX] {
        let mut task = Renderer::new()
            .unwrap()
            .preview_task(Arc::clone(&grid), options, Generation::new(11))
            .unwrap();
        while task.progress().completed() < task.progress().total() {
            task.advance(budget, Generation::new(11)).unwrap();
        }
        assert_eq!(task.finish(Generation::new(11)).unwrap(), expected);
    }
}

#[test]
fn block_tiles_are_seamless_at_full_and_zero_coverage() {
    let geometry = Renderer::new().unwrap().aspect_geometry(5, 30).unwrap();
    let cell_height = u32::from(geometry.source_cell_height);
    let options = mode_options(RenderMode::Block);
    let mut renderer = Renderer::new().unwrap();
    for (value, expected) in [(255, [0, 255, 0, 255]), (0, [0, 0, 0, 255])] {
        let grid = Sampler::new(source(10, cell_height * 2, move |_, _| {
            [value, value, value, 255]
        }))
        .sample_average_rect(5, cell_height)
        .unwrap();
        let frame = renderer.render(&grid, options).unwrap();
        assert!(frame
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel == expected));
    }
}

#[test]
fn block_tiles_are_uniform_and_use_only_coverage_levels() {
    let grid = gradient_grid(4, 3, true);
    let options = mode_options(RenderMode::Block);
    let frame = Renderer::new().unwrap().render(&grid, options).unwrap();
    let (tile_width, tile_height) = {
        let dimensions = Renderer::new()
            .unwrap()
            .frame_dimensions(&grid, options)
            .unwrap();
        (dimensions.0 / grid.width(), dimensions.1 / grid.height())
    };
    let stride = usize::try_from(frame.width()).unwrap() * 4;
    let mut levels = std::collections::BTreeSet::new();
    for row in 0..grid.height() {
        for column in 0..grid.width() {
            let first_y = usize::try_from(row * tile_height).unwrap();
            let first_x = usize::try_from(column * tile_width).unwrap();
            let first = (first_y * stride) + first_x * 4;
            let reference = &frame.pixels()[first..first + 4];
            levels.insert(reference[1]);
            for y in 0..usize::try_from(tile_height).unwrap() {
                for x in 0..usize::try_from(tile_width).unwrap() {
                    let index = (first_y + y) * stride + (first_x + x) * 4;
                    assert_eq!(&frame.pixels()[index..index + 4], reference);
                }
            }
        }
    }
    assert!(levels
        .iter()
        .all(|level| [0, 64, 128, 191, 255].contains(level)));
}

#[test]
fn ascii_space_entries_produce_background_only_tiles() {
    let grid = Sampler::new(source(10, 10, |_, _| [0, 0, 0, 255]))
        .sample_average_rect(
            5,
            u32::from(
                Renderer::new()
                    .unwrap()
                    .aspect_geometry(5, 30)
                    .unwrap()
                    .source_cell_height,
            ),
        )
        .unwrap();
    let ramp = Ramp::new(" .").unwrap();
    let frame = Renderer::new()
        .unwrap()
        .render(&grid, mode_options(RenderMode::Ascii(ramp)))
        .unwrap();
    assert!(frame
        .pixels()
        .chunks_exact(4)
        .all(|pixel| pixel == [0, 0, 0, 255]));
}

#[test]
fn ascii_and_block_reject_classic_sampling_and_binary_accepts_both() {
    let classic = mixed_grid(3, 3);
    let average = gradient_grid(3, 3, false);
    let mut renderer = Renderer::new().unwrap();
    for mode in [RenderMode::Ascii(Ramp::DEFAULT_ASCII), RenderMode::Block] {
        let options = RenderOptions { mode, ..options() };
        assert!(matches!(
            renderer.render(&classic, options),
            Err(RenderError::Contract(ContractError::InvalidSettings(_)))
        ));
        assert!(renderer.render_preview(&classic, options).is_err());
        assert!(renderer.render_rows(&classic, options, 0, 1).is_err());
    }
    let binary_classic = RenderOptions {
        mode: RenderMode::Binary,
        ..options()
    };
    assert!(renderer.render(&classic, binary_classic).is_ok());
    let binary_average = RenderOptions {
        sampling: Sampling::Average,
        mode: RenderMode::Binary,
        ..options()
    };
    assert!(renderer.render(&average, binary_average).is_ok());
}

#[test]
fn full_64_distinct_glyph_ramp_stays_within_glyph_cache_bound() {
    let text: String = (0x20_u8..0x60).map(char::from).collect();
    let ramp = Ramp::new(&text).unwrap();
    assert_eq!(ramp.len(), 64);
    let grid = gradient_grid(6, 3, false);
    let capacity = 4 * 1024;
    let mut renderer = Renderer::with_cache_capacity(capacity).unwrap();
    renderer
        .render(&grid, mode_options(RenderMode::Ascii(ramp)))
        .unwrap();
    let stats = renderer.cache_stats();
    assert!(stats.bytes() <= stats.capacity());
    assert_eq!(stats.capacity(), capacity);
    assert!(stats.entries() < 64);
}

#[test]
fn mode_changes_reuse_the_same_sampled_grid() {
    let grid = gradient_grid(3, 3, false);
    let mut renderer = Renderer::new().unwrap();
    let binary = renderer
        .render(&grid, mode_options(RenderMode::Binary))
        .unwrap();
    renderer
        .render(&grid, mode_options(RenderMode::Ascii(Ramp::DEFAULT_ASCII)))
        .unwrap();
    renderer
        .render(&grid, mode_options(RenderMode::Block))
        .unwrap();
    assert_eq!(
        renderer
            .render(&grid, mode_options(RenderMode::Binary))
            .unwrap(),
        binary
    );
}

fn colored_grid(
    columns: u32,
    rows: u32,
    partial: bool,
) -> Arc<image_to_binary_processing::SampleGrid> {
    let cell_height = u32::from(
        Renderer::new()
            .unwrap()
            .aspect_geometry(5, 30)
            .unwrap()
            .source_cell_height,
    );
    let (extra_width, extra_height) = if partial { (2, 3) } else { (0, 0) };
    let source = source(
        columns * 5 + extra_width,
        cell_height * rows + extra_height,
        |x, y| {
            let column = x / 5;
            let row = y / cell_height;
            let value = gray_cell_value(column, row);
            let alpha = match (column + row) % 3 {
                0 => 255,
                1 => 0,
                _ => 128,
            };
            [value, 255 - value, value / 2, alpha]
        },
    );
    Sampler::new(source)
        .sample_average_rect(5, cell_height)
        .unwrap()
}

fn rgb(red: u8, green: u8, blue: u8) -> Rgb {
    Rgb { red, green, blue }
}

fn monochrome_style() -> RenderStyle {
    RenderStyle {
        foreground: rgb(255, 128, 0),
        background: rgb(0, 0, 64),
        ..RenderStyle::default()
    }
}

fn source_style() -> RenderStyle {
    RenderStyle {
        color: ColorMode::Source,
        background: rgb(0, 0, 64),
        ..RenderStyle::default()
    }
}

const GOLDEN_MONOCHROME_CUSTOM: u64 = 6_427_888_498_122_285_736;
const GOLDEN_SOURCE_COLOR: u64 = 776_226_303_450_767_921;
const GOLDEN_CONTRAST: u64 = 4_527_450_082_118_090_894;
const GOLDEN_INVERT: u64 = 4_378_243_466_056_001_029;
const GOLDEN_THRESHOLD: u64 = 1_406_490_999_506_980_389;

fn golden_frame(grid: &image_to_binary_processing::SampleGrid, options: RenderOptions) -> u64 {
    let mut renderer = Renderer::new().unwrap();
    let first = renderer.render(grid, options).unwrap();
    let second = renderer.render(grid, options).unwrap();
    assert_eq!(first, second);
    assert!(first.pixels().chunks_exact(4).all(|pixel| pixel[3] == 255));
    fnv64(first.pixels())
}

#[test]
fn style_goldens_are_pinned_and_deterministic() {
    let gray = gradient_grid(3, 3, false);
    let colored = colored_grid(3, 3, false);
    let monochrome = RenderOptions {
        style: monochrome_style(),
        ..mode_options(RenderMode::Binary)
    };
    let source = RenderOptions {
        style: source_style(),
        ..mode_options(RenderMode::Ascii(Ramp::DEFAULT_ASCII))
    };
    let contrast = RenderOptions {
        style: RenderStyle {
            contrast: -40,
            ..RenderStyle::default()
        },
        ..mode_options(RenderMode::Ascii(Ramp::DEFAULT_ASCII))
    };
    let invert = RenderOptions {
        style: RenderStyle {
            invert: true,
            ..RenderStyle::default()
        },
        ..mode_options(RenderMode::Ascii(Ramp::new("-=+#").unwrap()))
    };
    let threshold = RenderOptions {
        threshold: 96,
        ..mode_options(RenderMode::Binary)
    };
    assert_eq!(
        golden_frame(&gray, monochrome),
        GOLDEN_MONOCHROME_CUSTOM,
        "monochrome"
    );
    assert_eq!(
        golden_frame(&colored, source),
        GOLDEN_SOURCE_COLOR,
        "source"
    );
    assert_eq!(golden_frame(&gray, contrast), GOLDEN_CONTRAST, "contrast");
    assert_eq!(golden_frame(&gray, invert), GOLDEN_INVERT, "invert");
    assert_eq!(
        golden_frame(&gray, threshold),
        GOLDEN_THRESHOLD,
        "threshold"
    );
}

#[test]
fn every_style_matches_row_batches_and_preview_with_partial_cells() {
    let grid = colored_grid(4, 6, true);
    assert_eq!(grid.height(), 7);
    let styles = [
        RenderStyle::default(),
        monochrome_style(),
        source_style(),
        RenderStyle {
            contrast: 60,
            invert: true,
            color: ColorMode::Source,
            background: rgb(5, 5, 5),
            ..RenderStyle::default()
        },
    ];
    let modes = [
        RenderMode::Binary,
        RenderMode::Ascii(Ramp::DEFAULT_ASCII),
        RenderMode::Block,
    ];
    let mut renderer = Renderer::new().unwrap();
    for style in styles {
        for mode in modes {
            let options = RenderOptions {
                style,
                ..mode_options(mode)
            };
            let full = renderer.render(&grid, options).unwrap();
            assert!(full.pixels().chunks_exact(4).all(|pixel| pixel[3] == 255));
            for batch in [1, 3, 4, 5] {
                let rows = render_batches(&mut renderer, &grid, options, batch);
                assert_eq!(rows, full.pixels(), "{style:?} {mode:?} batch {batch}");
            }
            assert_eq!(
                renderer.frame_dimensions(&grid, options).unwrap(),
                (full.width(), full.height())
            );
            assert_eq!(renderer.render_preview(&grid, options).unwrap(), full);
            let mut task = renderer
                .preview_task(Arc::clone(&grid), options, Generation::new(21))
                .unwrap();
            while task.progress().completed() < task.progress().total() {
                task.advance(9, Generation::new(21)).unwrap();
            }
            assert_eq!(task.finish(Generation::new(21)).unwrap(), full);
        }
    }
}

#[test]
fn source_color_opaque_cell_shows_exact_rgb_in_glyph_modes() {
    let grid = {
        let cell_height = u32::from(
            Renderer::new()
                .unwrap()
                .aspect_geometry(5, 30)
                .unwrap()
                .source_cell_height,
        );
        Sampler::new(source(5, cell_height, |_, _| [255, 230, 255, 255]))
            .sample_average_rect(5, cell_height)
            .unwrap()
    };
    let mut renderer = Renderer::new().unwrap();
    let style = RenderStyle {
        color: ColorMode::Source,
        background: rgb(0, 0, 0),
        ..RenderStyle::default()
    };
    for mode in [
        RenderMode::Binary,
        RenderMode::Ascii(Ramp::new("@").unwrap()),
    ] {
        let options = RenderOptions {
            style,
            ..mode_options(mode)
        };
        let frame = renderer.render(&grid, options).unwrap();
        let mut exact = 0;
        for pixel in frame.pixels().chunks_exact(4) {
            assert_eq!(pixel[3], 255, "{mode:?}");
            if pixel == [255, 230, 255, 255] {
                exact += 1;
            } else {
                assert_eq!(pixel[0], pixel[2], "{mode:?} {pixel:?}");
                assert!(pixel[1] <= 230, "{mode:?} {pixel:?}");
            }
        }
        assert!(
            exact > 0,
            "{mode:?} has no fully covered exact-colour pixel"
        );
    }
}

fn average_cell_grid_with(
    pixel: impl Fn(u32, u32) -> [u8; 4],
) -> Arc<image_to_binary_processing::SampleGrid> {
    let cell_height = u32::from(
        Renderer::new()
            .unwrap()
            .aspect_geometry(5, 30)
            .unwrap()
            .source_cell_height,
    );
    Sampler::new(source(5, cell_height, pixel))
        .sample_average_rect(5, cell_height)
        .unwrap()
}

#[test]
fn source_color_weights_hidden_transparent_colour_out_of_the_render() {
    let cell_height = u32::from(
        Renderer::new()
            .unwrap()
            .aspect_geometry(5, 30)
            .unwrap()
            .source_cell_height,
    );
    let cell_pixels = 5 * cell_height;
    let grid = average_cell_grid_with(|x, y| {
        let index = y * 5 + x;
        if index * 25 < cell_pixels * 12 {
            [255, 0, 0, 255]
        } else {
            [255, 255, 255, 0]
        }
    });
    let style = RenderStyle {
        color: ColorMode::Source,
        background: rgb(0, 0, 0),
        ..RenderStyle::default()
    };
    let mut renderer = Renderer::new().unwrap();
    for mode in [
        RenderMode::Binary,
        RenderMode::Ascii(Ramp::DEFAULT_ASCII),
        RenderMode::Block,
    ] {
        let frame = renderer
            .render(
                &grid,
                RenderOptions {
                    style,
                    ..mode_options(mode)
                },
            )
            .unwrap();
        assert!(frame
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel[1] == 0 && pixel[2] == 0 && pixel[3] == 255));
    }
}

#[test]
fn all_transparent_source_cell_renders_exact_background_in_source_mode() {
    let grid = average_cell_grid_with(|_, _| [255, 255, 255, 0]);
    let background = rgb(10, 20, 30);
    let frame = Renderer::new()
        .unwrap()
        .render(
            &grid,
            RenderOptions {
                style: RenderStyle {
                    color: ColorMode::Source,
                    background,
                    ..RenderStyle::default()
                },
                ..mode_options(RenderMode::Block)
            },
        )
        .unwrap();
    assert!(frame
        .pixels()
        .chunks_exact(4)
        .all(|pixel| pixel == [10, 20, 30, 255]));
}

#[test]
fn invalidation_policy_reuses_grids_and_masks_exactly() {
    let checker = |x: u32, y: u32| {
        if ((x / 5) + (y / 5)) % 2 == 0 {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        }
    };
    let image = source(15, 15, checker);
    let mut sampler = Sampler::new(Arc::clone(&image));
    let grid = sampler.sample(5, Sampling::Classic).unwrap();
    assert_eq!(sampler.cache_stats().computed(), 1);
    assert_eq!(sampler.cache_stats().entries(), 1);

    let mut renderer = Renderer::new().unwrap();
    renderer.render(&grid, options()).unwrap();
    let base = renderer.cache_stats();
    assert_eq!(base.rasterized(), 2);
    assert_eq!(base.entries(), 2);

    let tone_and_colour = [
        RenderOptions {
            style: RenderStyle {
                contrast: 30,
                ..RenderStyle::default()
            },
            ..options()
        },
        RenderOptions {
            style: RenderStyle {
                contrast: -100,
                ..RenderStyle::default()
            },
            ..options()
        },
        RenderOptions {
            style: RenderStyle {
                invert: true,
                ..RenderStyle::default()
            },
            ..options()
        },
        RenderOptions {
            threshold: 0,
            ..options()
        },
        RenderOptions {
            style: monochrome_style(),
            ..options()
        },
        RenderOptions {
            style: source_style(),
            ..options()
        },
    ];
    for changed in tone_and_colour {
        let again = sampler.sample(5, Sampling::Classic).unwrap();
        assert!(Arc::ptr_eq(&again, &grid));
        renderer.render(&again, changed).unwrap();
    }
    assert_eq!(sampler.cache_stats().computed(), 1);
    assert_eq!(renderer.cache_stats().rasterized(), base.rasterized());
    assert_eq!(renderer.cache_stats().entries(), base.entries());

    let other_size = RenderOptions {
        font_size: 20,
        ..options()
    };
    renderer.render(&grid, other_size).unwrap();
    assert_eq!(sampler.cache_stats().computed(), 1);
    assert_eq!(renderer.cache_stats().rasterized(), 4);
    assert_eq!(renderer.cache_stats().entries(), 4);

    let wider = sampler.sample(6, Sampling::Classic).unwrap();
    assert!(!Arc::ptr_eq(&wider, &grid));
    assert_eq!(sampler.cache_stats().computed(), 2);
    assert_eq!(sampler.cache_stats().entries(), 2);
    let average_height = u32::from(
        Renderer::new()
            .unwrap()
            .aspect_geometry(5, 30)
            .unwrap()
            .source_cell_height,
    );
    let average = sampler.sample_average_rect(5, average_height).unwrap();
    assert!(!Arc::ptr_eq(&average, &grid));
    assert_eq!(sampler.cache_stats().computed(), 3);

    let other = source(15, 15, |_, _| [255, 255, 255, 255]);
    let mut other_sampler = Sampler::new(other);
    let fresh = other_sampler.sample(5, Sampling::Classic).unwrap();
    assert_eq!(other_sampler.cache_stats().computed(), 1);
    assert_ne!(fresh.cell(0, 0), grid.cell(0, 0));
}

#[test]
fn mode_ramp_and_colour_changes_on_average_grid_reuse_grid_and_masks() {
    let grid = gradient_grid(3, 3, false);
    let mut renderer = Renderer::new().unwrap();
    renderer
        .render(&grid, mode_options(RenderMode::Binary))
        .unwrap();
    let base = renderer.cache_stats();
    assert_eq!(base.rasterized(), 2);
    let reversed = Ramp::new("10").unwrap();
    let changes = [
        mode_options(RenderMode::Ascii(reversed)),
        mode_options(RenderMode::Ascii(Ramp::new("01").unwrap())),
        mode_options(RenderMode::Block),
        RenderOptions {
            style: monochrome_style(),
            ..mode_options(RenderMode::Binary)
        },
        RenderOptions {
            style: RenderStyle {
                invert: true,
                contrast: 50,
                ..RenderStyle::default()
            },
            ..mode_options(RenderMode::Binary)
        },
    ];
    for options in changes {
        renderer.render(&grid, options).unwrap();
        assert_eq!(renderer.cache_stats().rasterized(), base.rasterized());
        assert_eq!(renderer.cache_stats().entries(), base.entries());
    }
    renderer
        .render(
            &grid,
            RenderOptions {
                font_size: 20,
                ..mode_options(RenderMode::Binary)
            },
        )
        .unwrap();
    assert_eq!(renderer.cache_stats().rasterized(), base.rasterized() * 2);
}
