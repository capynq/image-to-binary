//! Deterministic source-cell sampling and a private bounded grid cache.

use std::collections::VecDeque;
use std::mem::size_of;
use std::sync::Arc;

use image_to_binary_core::{ContractError, Sampling};

use crate::DecodedSource;

const DEFAULT_CACHE_BYTES: u64 = 64 * 1024 * 1024;

/// The sampled values for one source cell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SampleCell {
    luma: u8,
    color: [u8; 4],
}

impl SampleCell {
    /// Returns the integer luminance value, ignoring source alpha.
    #[must_use]
    pub const fn luma(self) -> u8 {
        self.luma
    }

    /// Returns the straight, averaged RGBA channels.
    #[must_use]
    pub const fn color(self) -> [u8; 4] {
        self.color
    }

    /// Maps this luminance to the classic binary byte without allocating text.
    #[must_use]
    pub const fn binary(self, threshold: u8) -> u8 {
        if self.luma < threshold {
            b'1'
        } else {
            b'0'
        }
    }
}

/// An immutable, row-major grid of sampled cells.
#[derive(Debug, Eq, PartialEq)]
pub struct SampleGrid {
    width: u32,
    height: u32,
    sampling: Sampling,
    source_cell_width: u8,
    source_cell_height: u32,
    cells: Vec<SampleCell>,
}

impl SampleGrid {
    /// Returns the number of cells horizontally.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the number of cells vertically.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the sampling mode used to construct this grid.
    #[must_use]
    pub const fn sampling(&self) -> Sampling {
        self.sampling
    }

    /// Returns the source-cell width used to construct this grid.
    #[must_use]
    pub const fn source_cell_width(&self) -> u8 {
        self.source_cell_width
    }

    /// Returns the source-cell height used to construct this grid.
    #[must_use]
    pub const fn source_cell_height(&self) -> u32 {
        self.source_cell_height
    }

    /// Returns a cell, or `None` outside the grid.
    #[must_use]
    pub fn cell(&self, x: u32, y: u32) -> Option<SampleCell> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = usize::try_from(y)
            .ok()?
            .checked_mul(usize::try_from(self.width).ok()?)?
            .checked_add(usize::try_from(x).ok()?)?;
        self.cells.get(index).copied()
    }

    /// Returns the number of retained cells.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Returns whether this grid has no cells.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CacheKey {
    cell_width: u8,
    cell_height: u32,
    sampling: Sampling,
}

#[derive(Debug)]
struct CacheEntry {
    key: CacheKey,
    grid: Arc<SampleGrid>,
    bytes: u64,
}

/// Owns one immutable source and caches its sampled grids by geometry.
///
/// The cache ceiling charges only internal cell-vector storage; source pixels,
/// grid metadata, `Arc` headers, and grids retained by callers are excluded.
#[derive(Debug)]
pub struct Sampler {
    source: Arc<DecodedSource>,
    cache: VecDeque<CacheEntry>,
    cache_bytes: u64,
    cache_capacity: u64,
}

impl Sampler {
    /// Creates a sampler with the default 64 MiB internal cache.
    #[must_use]
    pub fn new(source: Arc<DecodedSource>) -> Self {
        Self::with_cache_capacity(source, DEFAULT_CACHE_BYTES)
    }

    /// Creates a sampler with a caller-defined cache ceiling. Zero disables caching.
    #[must_use]
    pub fn with_cache_capacity(source: Arc<DecodedSource>, cache_capacity: u64) -> Self {
        Self {
            source,
            cache: VecDeque::new(),
            cache_bytes: 0,
            cache_capacity,
        }
    }

    /// Samples the source using the requested geometry and returns a shared grid.
    ///
    /// Classic geometry uses floor division and nearest-neighbor center points.
    /// Average geometry uses ceil division and includes partial edge cells.
    /// Threshold is intentionally absent: binary mapping can reuse each grid.
    ///
    /// # Errors
    ///
    /// Returns a contract error when geometry or allocation arithmetic fails.
    pub fn sample(
        &mut self,
        cell_size: u8,
        sampling: Sampling,
    ) -> Result<Arc<SampleGrid>, ContractError> {
        self.sample_with_geometry(cell_size, u32::from(cell_size), sampling)
    }

    /// Samples average cells with independently controlled source width and height.
    ///
    /// The width is bounded to the classic cell-size range and the height is
    /// bounded to a positive font-derived line-cell range. Partial edge cells
    /// are included, and the resulting grid is cached by both dimensions.
    ///
    /// # Errors
    ///
    /// Returns a contract error for invalid geometry or allocation arithmetic.
    pub fn sample_average_rect(
        &mut self,
        cell_width: u8,
        cell_height: u32,
    ) -> Result<Arc<SampleGrid>, ContractError> {
        if !(1..=1024).contains(&cell_height) {
            return Err(ContractError::InvalidSettings("cell_height"));
        }
        self.sample_with_geometry(cell_width, cell_height, Sampling::Average)
    }

    fn sample_with_geometry(
        &mut self,
        cell_width: u8,
        cell_height: u32,
        sampling: Sampling,
    ) -> Result<Arc<SampleGrid>, ContractError> {
        if !(5..=30).contains(&cell_width) {
            return Err(ContractError::InvalidSettings("cell_width"));
        }
        let key = CacheKey {
            cell_width,
            cell_height,
            sampling,
        };
        if let Some(position) = self.cache.iter().position(|entry| entry.key == key) {
            let entry = self
                .cache
                .remove(position)
                .ok_or(ContractError::ArithmeticOverflow)?;
            let grid = Arc::clone(&entry.grid);
            self.cache.push_back(entry);
            return Ok(grid);
        }
        let grid = Arc::new(build_grid(&self.source, cell_width, cell_height, sampling)?);
        let bytes = u64::try_from(grid.cells.capacity())
            .ok()
            .and_then(|count| count.checked_mul(u64::try_from(size_of::<SampleCell>()).ok()?))
            .ok_or(ContractError::ArithmeticOverflow)?;
        if self.cache_capacity != 0 && bytes <= self.cache_capacity {
            while self.cache_bytes > self.cache_capacity - bytes {
                if let Some(entry) = self.cache.pop_front() {
                    self.cache_bytes = self
                        .cache_bytes
                        .checked_sub(entry.bytes)
                        .ok_or(ContractError::ArithmeticOverflow)?;
                } else {
                    break;
                }
            }
            self.cache_bytes = self
                .cache_bytes
                .checked_add(bytes)
                .ok_or(ContractError::ArithmeticOverflow)?;
            self.cache.push_back(CacheEntry {
                key,
                grid: Arc::clone(&grid),
                bytes,
            });
        }
        Ok(grid)
    }
}

fn dimensions(
    source: &DecodedSource,
    cell_width: u8,
    cell_height: u32,
    sampling: Sampling,
) -> Result<(u32, u32), ContractError> {
    let source_dimensions = source.metadata().dimensions();
    let width_cell = u32::from(cell_width);
    let (width, height) = match sampling {
        Sampling::Classic => (
            source_dimensions.width() / width_cell,
            source_dimensions.height() / width_cell,
        ),
        Sampling::Average => (
            source_dimensions
                .width()
                .checked_add(width_cell - 1)
                .ok_or(ContractError::ArithmeticOverflow)?
                / width_cell,
            source_dimensions
                .height()
                .checked_add(cell_height - 1)
                .ok_or(ContractError::ArithmeticOverflow)?
                / cell_height,
        ),
    };
    Ok((width.max(1), height.max(1)))
}

fn build_grid(
    source: &DecodedSource,
    cell_width: u8,
    cell_height: u32,
    sampling: Sampling,
) -> Result<SampleGrid, ContractError> {
    let (width, height) = dimensions(source, cell_width, cell_height, sampling)?;
    let count = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(ContractError::ArithmeticOverflow)?;
    let count_usize = usize::try_from(count).map_err(|_| ContractError::ResourceLimitExceeded)?;
    let mut cells = Vec::new();
    cells
        .try_reserve_exact(count_usize)
        .map_err(|_| ContractError::ResourceLimitExceeded)?;
    let x_coordinates = if sampling == Sampling::Classic {
        Some(classic_coordinates(
            source.metadata().dimensions().width(),
            width,
        )?)
    } else {
        None
    };
    let y_coordinates = if sampling == Sampling::Classic {
        Some(classic_coordinates(
            source.metadata().dimensions().height(),
            height,
        )?)
    } else {
        None
    };
    for y in 0..height {
        for x in 0..width {
            cells.push(match sampling {
                Sampling::Classic => classic_cell(
                    source,
                    x_coordinates
                        .as_ref()
                        .ok_or(ContractError::ArithmeticOverflow)?
                        [usize::try_from(x).map_err(|_| ContractError::ArithmeticOverflow)?],
                    y_coordinates
                        .as_ref()
                        .ok_or(ContractError::ArithmeticOverflow)?
                        [usize::try_from(y).map_err(|_| ContractError::ArithmeticOverflow)?],
                )?,
                Sampling::Average => average_cell(source, x, y, cell_width, cell_height)?,
            });
        }
    }
    Ok(SampleGrid {
        width,
        height,
        sampling,
        source_cell_width: cell_width,
        source_cell_height: cell_height,
        cells,
    })
}

fn source_pixel(source: &DecodedSource, x: u32, y: u32) -> Result<[u8; 4], ContractError> {
    let source_width = usize::try_from(source.metadata().dimensions().width())
        .map_err(|_| ContractError::ArithmeticOverflow)?;
    let index = usize::try_from(y)
        .map_err(|_| ContractError::ArithmeticOverflow)?
        .checked_mul(source_width)
        .and_then(|value| value.checked_add(usize::try_from(x).ok()?))
        .and_then(|value| value.checked_mul(4))
        .ok_or(ContractError::ArithmeticOverflow)?;
    let pixel = source
        .pixels()
        .get(
            index
                ..index
                    .checked_add(4)
                    .ok_or(ContractError::ArithmeticOverflow)?,
        )
        .ok_or(ContractError::InvalidDimensions)?;
    Ok([pixel[0], pixel[1], pixel[2], pixel[3]])
}

fn luma(color: [u8; 4]) -> u8 {
    let value = u32::from(color[0]) * 19_595
        + u32::from(color[1]) * 38_470
        + u32::from(color[2]) * 7_471
        + 32_768;
    u8::try_from(value >> 16).unwrap_or(u8::MAX)
}

/// Computes Pillow-compatible positive-coordinate truncation from a validated grid.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn classic_coordinates(source_size: u32, grid_size: u32) -> Result<Vec<u32>, ContractError> {
    let mut coordinates = Vec::new();
    coordinates
        .try_reserve(usize::try_from(grid_size).map_err(|_| ContractError::ResourceLimitExceeded)?)
        .map_err(|_| ContractError::ResourceLimitExceeded)?;
    let scale = f64::from(source_size) / f64::from(grid_size);
    let mut coordinate = scale * 0.5;
    for _ in 0..grid_size {
        coordinates.push((coordinate as u32).min(source_size - 1));
        coordinate += scale;
    }
    Ok(coordinates)
}

fn classic_cell(
    source: &DecodedSource,
    source_x: u32,
    source_y: u32,
) -> Result<SampleCell, ContractError> {
    let color = source_pixel(source, source_x, source_y)?;
    Ok(SampleCell {
        luma: luma(color),
        color,
    })
}

fn average_cell(
    source: &DecodedSource,
    x: u32,
    y: u32,
    cell_width: u8,
    cell_height: u32,
) -> Result<SampleCell, ContractError> {
    let dimensions = source.metadata().dimensions();
    let left = x
        .checked_mul(u32::from(cell_width))
        .ok_or(ContractError::ArithmeticOverflow)?;
    let top = y
        .checked_mul(cell_height)
        .ok_or(ContractError::ArithmeticOverflow)?;
    let right = left
        .checked_add(u32::from(cell_width))
        .ok_or(ContractError::ArithmeticOverflow)?
        .min(dimensions.width());
    let bottom = top
        .checked_add(cell_height)
        .ok_or(ContractError::ArithmeticOverflow)?
        .min(dimensions.height());
    let mut sums = [0_u64; 4];
    let mut luma_sum = 0_u64;
    let mut count = 0_u64;
    for source_y in top..bottom {
        for source_x in left..right {
            let color = source_pixel(source, source_x, source_y)?;
            for (sum, channel) in sums.iter_mut().zip(color) {
                *sum = sum
                    .checked_add(u64::from(channel))
                    .ok_or(ContractError::ArithmeticOverflow)?;
            }
            luma_sum = luma_sum
                .checked_add(u64::from(luma(color)))
                .ok_or(ContractError::ArithmeticOverflow)?;
            count = count
                .checked_add(1)
                .ok_or(ContractError::ArithmeticOverflow)?;
        }
    }
    if count == 0 {
        return Err(ContractError::InvalidDimensions);
    }
    let color = [
        u8::try_from(sums[0] / count).map_err(|_| ContractError::ArithmeticOverflow)?,
        u8::try_from(sums[1] / count).map_err(|_| ContractError::ArithmeticOverflow)?,
        u8::try_from(sums[2] / count).map_err(|_| ContractError::ArithmeticOverflow)?,
        u8::try_from(sums[3] / count).map_err(|_| ContractError::ArithmeticOverflow)?,
    ];
    Ok(SampleCell {
        luma: u8::try_from(luma_sum / count).map_err(|_| ContractError::ArithmeticOverflow)?,
        color,
    })
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
    use std::io::Cursor;

    use super::*;
    use crate::{decode, DecodeLimits};
    use image_to_binary_core::Sampling;

    fn source(width: u32, height: u32, pixels: &[[u8; 4]]) -> Arc<DecodedSource> {
        let image = ImageBuffer::from_fn(width, height, |x, y| {
            Rgba(pixels[usize::try_from(y * width + x).unwrap()])
        });
        let mut output = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut output, ImageFormat::Png)
            .unwrap();
        Arc::new(decode(&output.into_inner(), DecodeLimits::default()).unwrap())
    }

    #[test]
    fn classic_uses_floor_grid_centers_and_tiny_clamp() {
        let mut sampler = Sampler::new(source(59, 1, &[[127, 0, 0, 255]; 59]));
        let grid = sampler.sample(20, Sampling::Classic).unwrap();
        assert_eq!((grid.width(), grid.height()), (2, 1));
        assert_eq!(grid.sampling(), Sampling::Classic);
        assert_eq!(grid.source_cell_width(), 20);
        assert_eq!(grid.source_cell_height(), 20);
        assert_eq!(grid.cell(0, 0).unwrap().color(), [127, 0, 0, 255]);
        let mut tiny = Sampler::new(source(1, 1, &[[255, 0, 0, 255]]));
        assert_eq!(
            (
                tiny.sample(5, Sampling::Classic).unwrap().width(),
                tiny.sample(5, Sampling::Classic).unwrap().height()
            ),
            (1, 1)
        );
    }

    #[test]
    fn classic_coordinate_recurrence_matches_width_twenty_eight() {
        let mut pixels = vec![[255, 255, 255, 255]; 28 * 28];
        pixels[13 * 28 + 13] = [0, 0, 0, 255];
        let mut sampler = Sampler::new(source(28, 28, &pixels));
        let grid = sampler.sample(5, Sampling::Classic).unwrap();
        assert_eq!((grid.width(), grid.height()), (5, 5));
        assert_eq!(grid.cell(2, 2).unwrap().binary(128), b'1');
        assert_eq!(grid.cell(3, 2).unwrap().binary(128), b'0');
        assert_eq!(grid.cell(2, 3).unwrap().binary(128), b'0');
    }

    #[test]
    fn luma_threshold_and_coefficients_are_exact() {
        let mut sampler = Sampler::new(source(1, 1, &[[127, 127, 127, 0]]));
        let grid = sampler.sample(5, Sampling::Average).unwrap();
        assert_eq!(grid.cell(0, 0).unwrap().binary(128), b'1');
        assert_eq!(grid.cell(0, 0).unwrap().luma(), 127);
        let mut red_sampler = Sampler::new(source(1, 1, &[[255, 0, 0, 255]]));
        let red = red_sampler.sample(5, Sampling::Average).unwrap();
        assert_eq!(red.cell(0, 0).unwrap().color()[0], 255);
    }

    #[test]
    fn average_includes_partial_edges_and_straight_alpha() {
        let mut sampler = Sampler::new(source(
            6,
            1,
            &[
                [10, 20, 30, 0],
                [20, 30, 40, 10],
                [30, 40, 50, 20],
                [40, 50, 60, 30],
                [50, 60, 70, 40],
                [200, 210, 220, 50],
            ],
        ));
        let grid = sampler.sample(5, Sampling::Average).unwrap();
        assert_eq!((grid.width(), grid.height()), (2, 1));
        assert_eq!(grid.cell(0, 0).unwrap().color(), [30, 40, 50, 20]);
        assert_eq!(grid.cell(1, 0).unwrap().color(), [200, 210, 220, 50]);
    }

    #[test]
    fn rectangular_average_uses_distinct_partial_regions() {
        let mut sampler = Sampler::new(source(
            6,
            3,
            &[
                [10, 0, 0, 255],
                [20, 0, 0, 255],
                [30, 0, 0, 255],
                [40, 0, 0, 255],
                [50, 0, 0, 255],
                [60, 0, 0, 255],
                [20, 0, 0, 255],
                [30, 0, 0, 255],
                [40, 0, 0, 255],
                [50, 0, 0, 255],
                [60, 0, 0, 255],
                [70, 0, 0, 255],
                [30, 0, 0, 255],
                [40, 0, 0, 255],
                [50, 0, 0, 255],
                [60, 0, 0, 255],
                [70, 0, 0, 255],
                [80, 0, 0, 255],
            ],
        ));
        let grid = sampler.sample_average_rect(5, 2).unwrap();
        assert_eq!((grid.width(), grid.height()), (2, 2));
        assert_eq!(grid.sampling(), Sampling::Average);
        assert_eq!(grid.source_cell_width(), 5);
        assert_eq!(grid.source_cell_height(), 2);
        assert_eq!(grid.cell(0, 0).unwrap().color(), [35, 0, 0, 255]);
        assert_eq!(grid.cell(1, 0).unwrap().color(), [65, 0, 0, 255]);
        assert_eq!(grid.cell(0, 1).unwrap().color(), [50, 0, 0, 255]);
        assert_eq!(grid.cell(1, 1).unwrap().color(), [80, 0, 0, 255]);
    }

    #[test]
    fn rectangular_average_cache_isolated_from_other_heights() {
        let source = source(20, 20, &[[1, 2, 3, 255]; 400]);
        let mut sampler = Sampler::new(Arc::clone(&source));
        let square = sampler.sample(5, Sampling::Average).unwrap();
        let rect = sampler.sample_average_rect(5, 6).unwrap();
        let square_again = sampler.sample_average_rect(5, 5).unwrap();
        assert!(!Arc::ptr_eq(&square, &rect));
        assert!(Arc::ptr_eq(&square, &square_again));
    }

    #[test]
    fn rectangular_average_rejects_invalid_geometry() {
        let mut sampler = Sampler::new(source(1, 1, &[[0, 0, 0, 255]]));
        assert!(sampler.sample_average_rect(4, 1).is_err());
        assert!(sampler.sample_average_rect(5, 0).is_err());
        assert!(sampler.sample_average_rect(5, 1025).is_err());
    }

    #[test]
    fn cache_reuses_geometry_but_isolates_sampling_and_capacity() {
        let source = source(20, 20, &[[1, 2, 3, 255]; 400]);
        let mut sampler = Sampler::with_cache_capacity(Arc::clone(&source), 128);
        let first = sampler.sample(5, Sampling::Classic).unwrap();
        let second = sampler.sample(5, Sampling::Classic).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        let average = sampler.sample(5, Sampling::Average).unwrap();
        assert!(!Arc::ptr_eq(&first, &average));
        let mut disabled = Sampler::with_cache_capacity(source, 0);
        let one = disabled.sample(5, Sampling::Classic).unwrap();
        let two = disabled.sample(5, Sampling::Classic).unwrap();
        assert!(!Arc::ptr_eq(&one, &two));
    }

    #[test]
    fn cache_capacity_boundary_and_eviction_are_deterministic() {
        let source = source(20, 20, &[[1, 2, 3, 255]; 400]);
        let cell_bytes = u64::try_from(size_of::<SampleCell>() * 16).unwrap();
        let mut below = Sampler::with_cache_capacity(Arc::clone(&source), cell_bytes - 1);
        let first = below.sample(5, Sampling::Classic).unwrap();
        let second = below.sample(5, Sampling::Classic).unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        let mut at_limit = Sampler::with_cache_capacity(Arc::clone(&source), cell_bytes);
        let first = at_limit.sample(5, Sampling::Classic).unwrap();
        let second = at_limit.sample(5, Sampling::Classic).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        let mut exact = Sampler::with_cache_capacity(Arc::clone(&source), cell_bytes * 2);
        let first = exact.sample(5, Sampling::Classic).unwrap();
        let second = exact.sample(5, Sampling::Classic).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        let average = exact.sample(5, Sampling::Average).unwrap();
        let classic_again = exact.sample(5, Sampling::Classic).unwrap();
        assert!(Arc::ptr_eq(&first, &classic_again));
        let _third_key = exact.sample(10, Sampling::Classic).unwrap();
        let classic_still_present = exact.sample(5, Sampling::Classic).unwrap();
        assert!(Arc::ptr_eq(&first, &classic_still_present));
        let average_again = exact.sample(5, Sampling::Average).unwrap();
        assert!(!Arc::ptr_eq(&average, &average_again));
    }
}
