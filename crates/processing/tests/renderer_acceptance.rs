use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
use image_to_binary_core::Sampling;
use image_to_binary_processing::{decode, DecodeLimits, RenderOptions, Renderer, Sampler};
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
