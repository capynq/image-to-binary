//! Deterministic source-cell sampling and a private bounded grid cache.

use std::collections::VecDeque;
use std::mem::size_of;
use std::sync::Arc;

use image_to_binary_core::{Cancellation, Generation, Progress};
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

    /// Creates a resumable sampling task without consulting or mutating the cache.
    ///
    /// # Errors
    ///
    /// Returns a settings, dimensions, arithmetic, or allocation error.
    pub fn task(
        &self,
        cell_width: u8,
        cell_height: u32,
        sampling: Sampling,
        generation: Generation,
    ) -> Result<SamplingTask, ContractError> {
        SamplingTask::new(
            Arc::clone(&self.source),
            cell_width,
            cell_height,
            sampling,
            generation,
        )
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
        let mut task = self.task(cell_width, cell_height, sampling, Generation::new(0))?;
        while matches!(
            task.advance(u64::MAX, Generation::new(0))?,
            SamplingAdvance::InProgress(_)
        ) {}
        let grid = Arc::new(task.finish(Generation::new(0))?);
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

/// Result of one bounded sampling call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SamplingAdvance {
    InProgress(Progress),
    Completed(Progress),
}

/// Resumable source sampler. Each budget unit reads exactly one source pixel.
#[derive(Debug)]
pub struct SamplingTask {
    source: Arc<DecodedSource>,
    width: u32,
    height: u32,
    cell_width: u8,
    cell_height: u32,
    sampling: Sampling,
    cancellation: Cancellation,
    progress: Progress,
    cells: Vec<SampleCell>,
    x: u32,
    y: u32,
    pixel_x: u32,
    pixel_y: u32,
    bounds: (u32, u32, u32, u32),
    sums: [u64; 4],
    luma_sum: u64,
    count: u64,
    x_coordinate: f64,
    y_coordinate: f64,
    x_scale: f64,
    y_scale: f64,
    failed: Option<ContractError>,
}

impl SamplingTask {
    /// Creates a task after validating geometry and reserving its output grid.
    ///
    /// Allocation and grid setup happen synchronously and cannot be preempted.
    ///
    /// # Errors
    ///
    /// Returns a settings, dimensions, arithmetic, or allocation error.
    pub fn new(
        source: Arc<DecodedSource>,
        cell_width: u8,
        cell_height: u32,
        sampling: Sampling,
        generation: Generation,
    ) -> Result<Self, ContractError> {
        if !(5..=30).contains(&cell_width) {
            return Err(ContractError::InvalidSettings("cell_width"));
        }
        if !(1..=1024).contains(&cell_height) {
            return Err(ContractError::InvalidSettings("cell_height"));
        }
        if sampling == Sampling::Classic && u32::from(cell_width) != cell_height {
            return Err(ContractError::InvalidSettings("classic cell geometry"));
        }
        let (width, height) = dimensions(&source, cell_width, cell_height, sampling)?;
        let count = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(ContractError::ArithmeticOverflow)?;
        let count_usize =
            usize::try_from(count).map_err(|_| ContractError::ResourceLimitExceeded)?;
        let mut cells = Vec::new();
        cells
            .try_reserve_exact(count_usize)
            .map_err(|_| ContractError::ResourceLimitExceeded)?;
        let dims = source.metadata().dimensions();
        let total = match sampling {
            Sampling::Classic => count,
            Sampling::Average => u64::from(dims.width())
                .checked_mul(u64::from(dims.height()))
                .ok_or(ContractError::ArithmeticOverflow)?,
        };
        let x_scale = f64::from(dims.width()) / f64::from(width);
        let y_scale = f64::from(dims.height()) / f64::from(height);
        let x_coordinate = x_scale * 0.5;
        let y_coordinate = y_scale * 0.5;
        let mut task = Self {
            source,
            width,
            height,
            cell_width,
            cell_height,
            sampling,
            cancellation: Cancellation::new(generation),
            progress: Progress::new(total)?,
            cells,
            x: 0,
            y: 0,
            pixel_x: 0,
            pixel_y: 0,
            bounds: (0, 0, 0, 0),
            sums: [0; 4],
            luma_sum: 0,
            count: 0,
            x_coordinate,
            y_coordinate,
            x_scale,
            y_scale,
            failed: None,
        };
        task.set_cell_bounds()?;
        Ok(task)
    }

    /// Returns a snapshot of source pixel reads completed and the total.
    #[must_use]
    pub const fn progress(&self) -> Progress {
        self.progress
    }

    /// Requests cancellation; the next boundary check discards partial output.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    /// Reads at most `pixel_budget` source pixels and reports whether work is done.
    ///
    /// A zero budget is rejected without changing progress. Stale or cancelled
    /// tasks become terminal and release their partial cells.
    ///
    /// # Errors
    ///
    /// Returns an invalid-progress, cancellation, stale-generation, or sampling
    /// arithmetic error. Cancellation and stale-generation errors invalidate
    /// the task permanently.
    pub fn advance(
        &mut self,
        pixel_budget: u64,
        current_generation: Generation,
    ) -> Result<SamplingAdvance, ContractError> {
        if pixel_budget == 0 {
            return Err(ContractError::InvalidProgress);
        }
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        if let Err(error) = self.cancellation.check(current_generation) {
            self.failed = Some(error.clone());
            self.cells = Vec::new();
            return Err(error);
        }
        let result = self.advance_inner(pixel_budget);
        if let Err(error) = self.cancellation.check(current_generation) {
            self.failed = Some(error.clone());
            self.cells = Vec::new();
            return Err(error);
        }
        if let Err(error) = result {
            self.failed = Some(error.clone());
            self.cells = Vec::new();
            return Err(error);
        }
        Ok(if self.progress.completed() == self.progress.total() {
            SamplingAdvance::Completed(self.progress)
        } else {
            SamplingAdvance::InProgress(self.progress)
        })
    }

    fn advance_inner(&mut self, budget: u64) -> Result<(), ContractError> {
        let mut reads = 0;
        while reads < budget && self.progress.completed() < self.progress.total() {
            let (source_x, source_y) = if self.sampling == Sampling::Classic {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let sx =
                    (self.x_coordinate as u32).min(self.source.metadata().dimensions().width() - 1);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let sy = (self.y_coordinate as u32)
                    .min(self.source.metadata().dimensions().height() - 1);
                (sx, sy)
            } else {
                (self.pixel_x, self.pixel_y)
            };
            let color = source_pixel(&self.source, source_x, source_y)?;
            if self.sampling == Sampling::Classic {
                self.cells.push(SampleCell {
                    luma: luma(color),
                    color,
                });
                self.x += 1;
                self.x_coordinate += self.x_scale;
                if self.x == self.width {
                    self.x = 0;
                    self.x_coordinate = self.x_scale * 0.5;
                    self.y += 1;
                    self.y_coordinate += self.y_scale;
                }
            } else {
                for (sum, channel) in self.sums.iter_mut().zip(color) {
                    *sum = sum
                        .checked_add(u64::from(channel))
                        .ok_or(ContractError::ArithmeticOverflow)?;
                }
                self.luma_sum = self
                    .luma_sum
                    .checked_add(u64::from(luma(color)))
                    .ok_or(ContractError::ArithmeticOverflow)?;
                self.count = self
                    .count
                    .checked_add(1)
                    .ok_or(ContractError::ArithmeticOverflow)?;
                self.pixel_x += 1;
                if self.pixel_x == self.bounds.2 {
                    self.pixel_x = self.bounds.0;
                    self.pixel_y += 1;
                }
                if self.pixel_y == self.bounds.3 {
                    let color = [
                        u8::try_from(self.sums[0] / self.count)
                            .map_err(|_| ContractError::ArithmeticOverflow)?,
                        u8::try_from(self.sums[1] / self.count)
                            .map_err(|_| ContractError::ArithmeticOverflow)?,
                        u8::try_from(self.sums[2] / self.count)
                            .map_err(|_| ContractError::ArithmeticOverflow)?,
                        u8::try_from(self.sums[3] / self.count)
                            .map_err(|_| ContractError::ArithmeticOverflow)?,
                    ];
                    self.cells.push(SampleCell {
                        luma: u8::try_from(self.luma_sum / self.count)
                            .map_err(|_| ContractError::ArithmeticOverflow)?,
                        color,
                    });
                    self.sums = [0; 4];
                    self.luma_sum = 0;
                    self.count = 0;
                    self.x += 1;
                    if self.x == self.width {
                        self.x = 0;
                        self.y += 1;
                    }
                    if self.y < self.height {
                        self.set_cell_bounds()?;
                    }
                }
            }
            reads += 1;
            self.progress.advance(1)?;
        }
        Ok(())
    }

    fn set_cell_bounds(&mut self) -> Result<(), ContractError> {
        let dims = self.source.metadata().dimensions();
        let left = self
            .x
            .checked_mul(u32::from(self.cell_width))
            .ok_or(ContractError::ArithmeticOverflow)?;
        let top = self
            .y
            .checked_mul(self.cell_height)
            .ok_or(ContractError::ArithmeticOverflow)?;
        let right = left
            .checked_add(u32::from(self.cell_width))
            .ok_or(ContractError::ArithmeticOverflow)?
            .min(dims.width());
        let bottom = top
            .checked_add(self.cell_height)
            .ok_or(ContractError::ArithmeticOverflow)?
            .min(dims.height());
        self.bounds = (left, top, right, bottom);
        self.pixel_x = left;
        self.pixel_y = top;
        Ok(())
    }

    /// Consumes a completed task and returns its grid after generation checks.
    ///
    /// # Errors
    ///
    /// Returns cancellation, stale-generation, or incomplete-progress errors.
    pub fn finish(mut self, current_generation: Generation) -> Result<SampleGrid, ContractError> {
        if let Some(error) = self.failed.take() {
            return Err(error);
        }
        self.cancellation.check(current_generation)?;
        if self.progress.completed() != self.progress.total() {
            return Err(ContractError::InvalidProgress);
        }
        Ok(SampleGrid {
            width: self.width,
            height: self.height,
            sampling: self.sampling,
            source_cell_width: self.cell_width,
            source_cell_height: self.cell_height,
            cells: self.cells,
        })
    }
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
        let source = source(
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
        );
        let mut sampler = Sampler::new(Arc::clone(&source));
        let grid = sampler.sample_average_rect(5, 2).unwrap();
        assert_eq!((grid.width(), grid.height()), (2, 2));
        assert_eq!(grid.sampling(), Sampling::Average);
        assert_eq!(grid.source_cell_width(), 5);
        assert_eq!(grid.source_cell_height(), 2);
        assert_eq!(grid.cell(0, 0).unwrap().color(), [35, 0, 0, 255]);
        assert_eq!(grid.cell(1, 0).unwrap().color(), [65, 0, 0, 255]);
        assert_eq!(grid.cell(0, 1).unwrap().color(), [50, 0, 0, 255]);
        assert_eq!(grid.cell(1, 1).unwrap().color(), [80, 0, 0, 255]);
        for budget in [1, 17, 4096, u64::MAX] {
            let mut task = sampler
                .task(5, 2, Sampling::Average, Generation::new(11))
                .unwrap();
            loop {
                if matches!(
                    task.advance(budget, Generation::new(11)).unwrap(),
                    SamplingAdvance::Completed(_)
                ) {
                    break;
                }
            }
            let chunked = task.finish(Generation::new(11)).unwrap();
            assert_eq!(chunked.cell(0, 0).unwrap().color(), [35, 0, 0, 255]);
            assert_eq!(chunked.cell(1, 0).unwrap().color(), [65, 0, 0, 255]);
            assert_eq!(chunked.cell(0, 1).unwrap().color(), [50, 0, 0, 255]);
            assert_eq!(chunked.cell(1, 1).unwrap().color(), [80, 0, 0, 255]);
        }
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

    #[test]
    fn chunked_and_synchronous_sampling_are_identical() {
        let pixels: Vec<[u8; 4]> = (0_u32..28 * 19)
            .map(|index| {
                let x = index % 28;
                let y = index / 28;
                [
                    u8::try_from((x * 9 + y * 3) % 256).unwrap(),
                    u8::try_from((x * 2 + y * 13) % 256).unwrap(),
                    u8::try_from((x * 7 + y * 5) % 256).unwrap(),
                    u8::try_from((x * 11 + y * 17) % 256).unwrap(),
                ]
            })
            .collect();
        let gradient_source = source(28, 19, &pixels);
        for (sampling, cell_height) in [(Sampling::Classic, 5), (Sampling::Average, 3)] {
            let mut sampler = Sampler::new(Arc::clone(&gradient_source));
            let expected = sampler
                .sample_with_geometry(5, cell_height, sampling)
                .unwrap();
            for budget in [1, 17, 4096, u64::MAX] {
                let mut task = sampler
                    .task(5, cell_height, sampling, Generation::new(9))
                    .unwrap();
                loop {
                    if matches!(
                        task.advance(budget, Generation::new(9)).unwrap(),
                        SamplingAdvance::Completed(_)
                    ) {
                        break;
                    }
                }
                assert_eq!(*expected, task.finish(Generation::new(9)).unwrap());
            }
        }
        let narrow = source(
            6,
            1,
            &[
                [10, 20, 30, 40],
                [50, 60, 70, 80],
                [90, 100, 110, 120],
                [130, 140, 150, 160],
                [170, 180, 190, 200],
                [210, 220, 230, 240],
            ],
        );
        for (source, expected) in [
            (Arc::clone(&narrow), [130, 140, 150, 160]),
            (source(1, 1, &[[9, 19, 29, 39]]), [9, 19, 29, 39]),
        ] {
            let mut task =
                SamplingTask::new(source, 5, 5, Sampling::Classic, Generation::new(10)).unwrap();
            assert!(matches!(
                task.advance(1, Generation::new(10)).unwrap(),
                SamplingAdvance::Completed(_)
            ));
            assert_eq!(
                task.finish(Generation::new(10))
                    .unwrap()
                    .cell(0, 0)
                    .unwrap()
                    .color(),
                expected
            );
        }
    }

    #[test]
    fn average_budget_pauses_inside_a_large_cell() {
        let pixels: Vec<[u8; 4]> = (0_u32..30 * 1024)
            .map(|index| {
                [
                    u8::try_from(index % 256).unwrap(),
                    u8::try_from((index * 3 + 1) % 256).unwrap(),
                    u8::try_from((index * 7 + 2) % 256).unwrap(),
                    u8::try_from((index * 11 + 3) % 256).unwrap(),
                ]
            })
            .collect();
        let mut channel_sums = [0_u64; 4];
        let mut luma_sum = 0_u64;
        for pixel in &pixels {
            for (sum, channel) in channel_sums.iter_mut().zip(pixel) {
                *sum += u64::from(*channel);
            }
            let weighted = u32::from(pixel[0]) * 19_595
                + u32::from(pixel[1]) * 38_470
                + u32::from(pixel[2]) * 7_471
                + 32_768;
            luma_sum += u64::from(u8::try_from(weighted >> 16).unwrap());
        }
        let mut task = SamplingTask::new(
            source(30, 1024, &pixels),
            30,
            1024,
            Sampling::Average,
            Generation::new(1),
        )
        .unwrap();
        assert!(
            matches!(task.advance(17, Generation::new(1)).unwrap(), SamplingAdvance::InProgress(progress) if progress.completed() == 17 && progress.total() == 30 * 1024)
        );
        assert!(
            matches!(task.advance(1, Generation::new(1)).unwrap(), SamplingAdvance::InProgress(progress) if progress.completed() == 18 && progress.total() == 30 * 1024)
        );
        while matches!(
            task.advance(4096, Generation::new(1)).unwrap(),
            SamplingAdvance::InProgress(_)
        ) {}
        let grid = task.finish(Generation::new(1)).unwrap();
        assert_eq!((grid.width(), grid.height()), (1, 1));
        let cell = grid.cell(0, 0).unwrap();
        let count = u64::try_from(pixels.len()).unwrap();
        assert_eq!(
            cell.color(),
            [
                u8::try_from(channel_sums[0] / count).unwrap(),
                u8::try_from(channel_sums[1] / count).unwrap(),
                u8::try_from(channel_sums[2] / count).unwrap(),
                u8::try_from(channel_sums[3] / count).unwrap(),
            ]
        );
        assert_eq!(cell.luma(), u8::try_from(luma_sum / count).unwrap());
    }

    #[test]
    fn stale_cancelled_and_incomplete_tasks_never_yield_a_grid() {
        let source = source(30, 30, &[[0, 0, 0, 255]; 900]);
        let mut stale = SamplingTask::new(
            Arc::clone(&source),
            5,
            5,
            Sampling::Average,
            Generation::new(2),
        )
        .unwrap();
        stale.advance(1, Generation::new(2)).unwrap();
        assert_eq!(
            stale.advance(1, Generation::new(3)),
            Err(ContractError::StaleGeneration)
        );
        assert_eq!(stale.cells.capacity(), 0);
        assert_eq!(
            stale.advance(1, Generation::new(2)),
            Err(ContractError::StaleGeneration)
        );
        assert_eq!(
            stale.finish(Generation::new(2)),
            Err(ContractError::StaleGeneration)
        );
        let mut cancelled = SamplingTask::new(
            Arc::clone(&source),
            5,
            5,
            Sampling::Average,
            Generation::new(4),
        )
        .unwrap();
        cancelled.cancel();
        assert_eq!(
            cancelled.advance(1, Generation::new(4)),
            Err(ContractError::Cancelled)
        );
        assert_eq!(cancelled.cells.capacity(), 0);
        let mut partial_cancelled = SamplingTask::new(
            Arc::clone(&source),
            5,
            5,
            Sampling::Average,
            Generation::new(7),
        )
        .unwrap();
        partial_cancelled.advance(1, Generation::new(7)).unwrap();
        partial_cancelled.cancel();
        assert_eq!(
            partial_cancelled.advance(1, Generation::new(7)),
            Err(ContractError::Cancelled)
        );
        assert_eq!(partial_cancelled.cells.capacity(), 0);
        let mut complete_then_cancel = SamplingTask::new(
            Arc::clone(&source),
            5,
            5,
            Sampling::Classic,
            Generation::new(6),
        )
        .unwrap();
        assert!(matches!(
            complete_then_cancel
                .advance(u64::MAX, Generation::new(6))
                .unwrap(),
            SamplingAdvance::Completed(_)
        ));
        complete_then_cancel.cancel();
        assert_eq!(
            complete_then_cancel.finish(Generation::new(6)),
            Err(ContractError::Cancelled)
        );
        let incomplete =
            SamplingTask::new(source, 5, 5, Sampling::Classic, Generation::new(5)).unwrap();
        assert_eq!(
            incomplete.finish(Generation::new(5)),
            Err(ContractError::InvalidProgress)
        );
    }

    #[test]
    fn task_rejects_invalid_geometry_and_zero_budget() {
        let source = source(30, 30, &[[0, 0, 0, 255]; 900]);
        assert!(SamplingTask::new(
            Arc::clone(&source),
            4,
            4,
            Sampling::Classic,
            Generation::new(0)
        )
        .is_err());
        assert!(SamplingTask::new(
            Arc::clone(&source),
            5,
            6,
            Sampling::Classic,
            Generation::new(0)
        )
        .is_err());
        assert!(SamplingTask::new(
            Arc::clone(&source),
            5,
            1025,
            Sampling::Average,
            Generation::new(0)
        )
        .is_err());
        let mut task =
            SamplingTask::new(source, 5, 5, Sampling::Average, Generation::new(0)).unwrap();
        assert_eq!(
            task.advance(0, Generation::new(0)),
            Err(ContractError::InvalidProgress)
        );
        assert_eq!(task.progress().completed(), 0);
    }
}
