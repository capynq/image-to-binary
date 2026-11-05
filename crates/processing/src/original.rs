//! Bounded, deterministic, resumable preview of the normalized original image.

use std::sync::Arc;

use image_to_binary_core::{Cancellation, ContractError, Generation, Progress};

use crate::{DecodedSource, PreviewAdvance, RenderedFrame};

/// Longest edge, in pixels, of an original preview. Smaller sources are never upscaled.
pub const ORIGINAL_PREVIEW_MAX_EDGE: u32 = 1600;

const MAX_OUTPUT_BYTES: u64 = 1600 * 1600 * 4;

fn output_dimensions(width: u32, height: u32) -> Result<(u32, u32), ContractError> {
    if width == 0 || height == 0 {
        return Err(ContractError::InvalidDimensions);
    }
    let longest = u64::from(width.max(height));
    let edge = u64::from(ORIGINAL_PREVIEW_MAX_EDGE);
    if longest <= edge {
        return Ok((width, height));
    }
    let scale = |value: u32| -> Result<u32, ContractError> {
        let scaled = u64::from(value)
            .checked_mul(edge)
            .ok_or(ContractError::ArithmeticOverflow)?
            / longest;
        u32::try_from(scaled.max(1)).map_err(|_| ContractError::ArithmeticOverflow)
    };
    Ok((scale(width)?, scale(height)?))
}

fn source_coordinate(output: u32, output_len: u32, source_len: u32) -> Result<u32, ContractError> {
    let numerator = u64::from(output)
        .checked_mul(2)
        .and_then(|value| value.checked_add(1))
        .and_then(|value| value.checked_mul(u64::from(source_len)))
        .ok_or(ContractError::ArithmeticOverflow)?;
    let denominator = u64::from(output_len)
        .checked_mul(2)
        .ok_or(ContractError::ArithmeticOverflow)?;
    let index = (numerator / denominator).min(u64::from(source_len).saturating_sub(1));
    u32::try_from(index).map_err(|_| ContractError::ArithmeticOverflow)
}

/// Resumable nearest-center downscale of an immutable normalized RGBA8 source.
///
/// The output longest edge is at most [`ORIGINAL_PREVIEW_MAX_EDGE`]; sources within
/// it keep their size. Scaled dimensions are floored with a minimum of one pixel.
/// Output pixel `(x, y)` copies source pixel `(floor((2x+1)*w/(2*ow)), floor((2y+1)*h/(2*oh)))`
/// unchanged, so alpha and straight colour are preserved exactly. One budget unit
/// writes one output pixel. The task shares the source through an `Arc`, never copies
/// it, and owns one output buffer of at most 1600 * 1600 * 4 bytes.
#[derive(Debug)]
pub struct OriginalPreviewTask {
    source: Arc<DecodedSource>,
    width: u32,
    height: u32,
    cancellation: Cancellation,
    progress: Progress,
    pixels: Vec<u8>,
    x: u32,
    y: u32,
    failed: Option<ContractError>,
}

impl OriginalPreviewTask {
    /// Validates geometry and fallibly reserves the whole output buffer.
    ///
    /// Reservation is synchronous and cannot be preempted.
    ///
    /// # Errors
    ///
    /// Returns an invalid-dimensions error for a zero or inconsistent source, an
    /// arithmetic error, or a resource-limit error when reservation fails.
    pub fn new(source: Arc<DecodedSource>, generation: Generation) -> Result<Self, ContractError> {
        let dimensions = source.metadata().dimensions();
        let (width, height) = output_dimensions(dimensions.width(), dimensions.height())?;
        let count = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(ContractError::ArithmeticOverflow)?;
        let bytes = count
            .checked_mul(4)
            .ok_or(ContractError::ArithmeticOverflow)?;
        if bytes > MAX_OUTPUT_BYTES {
            return Err(ContractError::ResourceLimitExceeded);
        }
        let source_bytes = u64::from(dimensions.width())
            .checked_mul(u64::from(dimensions.height()))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(ContractError::ArithmeticOverflow)?;
        if u64::try_from(source.pixels().len()).ok() != Some(source_bytes) {
            return Err(ContractError::InvalidDimensions);
        }
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(
                usize::try_from(bytes).map_err(|_| ContractError::ResourceLimitExceeded)?,
            )
            .map_err(|_| ContractError::ResourceLimitExceeded)?;
        Ok(Self {
            source,
            width,
            height,
            cancellation: Cancellation::new(generation),
            progress: Progress::new(count)?,
            pixels,
            x: 0,
            y: 0,
            failed: None,
        })
    }

    /// Returns a snapshot of output pixels written and the total.
    #[must_use]
    pub const fn progress(&self) -> Progress {
        self.progress
    }

    /// Returns the output width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the output height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Requests cancellation; the next boundary check discards partial output.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    /// Writes at most `pixel_budget` output pixels and reports whether work is done.
    ///
    /// A zero budget is rejected without mutating the task. Cancellation and the
    /// current generation are checked before and after the chunk.
    ///
    /// # Errors
    ///
    /// Returns an invalid-progress, cancellation, stale-generation, or arithmetic
    /// error. Every error except a zero budget is terminal and releases partial pixels.
    pub fn advance(
        &mut self,
        pixel_budget: u64,
        current_generation: Generation,
    ) -> Result<PreviewAdvance, ContractError> {
        if pixel_budget == 0 {
            return Err(ContractError::InvalidProgress);
        }
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        if let Err(error) = self.cancellation.check(current_generation) {
            return Err(self.fail(error));
        }
        let result = self.advance_inner(pixel_budget);
        if let Err(error) = self.cancellation.check(current_generation) {
            return Err(self.fail(error));
        }
        if let Err(error) = result {
            return Err(self.fail(error));
        }
        Ok(if self.progress.completed() == self.progress.total() {
            PreviewAdvance::Completed(self.progress)
        } else {
            PreviewAdvance::InProgress(self.progress)
        })
    }

    fn fail(&mut self, error: ContractError) -> ContractError {
        self.failed = Some(error.clone());
        self.pixels = Vec::new();
        error
    }

    fn advance_inner(&mut self, budget: u64) -> Result<(), ContractError> {
        let dimensions = self.source.metadata().dimensions();
        let source_width =
            usize::try_from(dimensions.width()).map_err(|_| ContractError::ArithmeticOverflow)?;
        let mut reads = 0;
        while reads < budget && self.progress.completed() < self.progress.total() {
            let source_x = source_coordinate(self.x, self.width, dimensions.width())?;
            let source_y = source_coordinate(self.y, self.height, dimensions.height())?;
            let offset = usize::try_from(source_y)
                .ok()
                .and_then(|row| row.checked_mul(source_width))
                .and_then(|index| index.checked_add(usize::try_from(source_x).ok()?))
                .and_then(|index| index.checked_mul(4))
                .ok_or(ContractError::ArithmeticOverflow)?;
            let end = offset
                .checked_add(4)
                .ok_or(ContractError::ArithmeticOverflow)?;
            let pixel = self
                .source
                .pixels()
                .get(offset..end)
                .ok_or(ContractError::InvalidDimensions)?;
            self.pixels.extend_from_slice(pixel);
            self.x += 1;
            if self.x == self.width {
                self.x = 0;
                self.y += 1;
            }
            reads += 1;
            self.progress.advance(1)?;
        }
        Ok(())
    }

    /// Consumes a completed task and returns its owned frame without copying pixels.
    ///
    /// # Errors
    ///
    /// Returns the terminal failure, a cancellation or stale-generation error, or an
    /// invalid-progress error when work is incomplete.
    pub fn finish(
        mut self,
        current_generation: Generation,
    ) -> Result<RenderedFrame, ContractError> {
        if let Some(error) = self.failed.take() {
            return Err(error);
        }
        self.cancellation.check(current_generation)?;
        if self.progress.completed() != self.progress.total() {
            return Err(ContractError::InvalidProgress);
        }
        RenderedFrame::from_rgba(self.width, self.height, self.pixels)
    }
}

#[cfg(test)]
mod tests {
    use image_to_binary_core::Dimensions;

    use super::*;
    use crate::DecodeLimits;

    fn pattern(width: u32, height: u32) -> Vec<u8> {
        let mut pixels = Vec::new();
        for y in 0..height {
            for x in 0..width {
                for channel in [
                    x * 7 + y * 3,
                    x * 2 + y * 13 + 1,
                    x * 5 + y * 11 + 2,
                    x * 11 + y * 17,
                ] {
                    pixels.push(u8::try_from(channel % 256).unwrap());
                }
            }
        }
        pixels
    }

    fn source(width: u32, height: u32) -> Arc<DecodedSource> {
        Arc::new(
            DecodedSource::from_rgba(
                Dimensions::new(width, height).unwrap(),
                pattern(width, height),
                DecodeLimits::default(),
            )
            .unwrap(),
        )
    }

    fn nearest(output: u32, output_len: u32, source_len: u32) -> usize {
        let found = (0..source_len)
            .filter(|candidate| {
                u64::from(*candidate) * 2 * u64::from(output_len)
                    <= (u64::from(output) * 2 + 1) * u64::from(source_len)
            })
            .max()
            .unwrap();
        usize::try_from(found).unwrap()
    }

    fn oracle(pixels: &[u8], width: u32, out_width: u32, height: u32, out_height: u32) -> Vec<u8> {
        let columns: Vec<usize> = (0..out_width)
            .map(|x| nearest(x, out_width, width))
            .collect();
        let rows: Vec<usize> = (0..out_height)
            .map(|y| nearest(y, out_height, height))
            .collect();
        let stride = usize::try_from(width).unwrap();
        let mut output = Vec::new();
        for row in &rows {
            for column in &columns {
                let from = (row * stride + column) * 4;
                output.extend_from_slice(&pixels[from..from + 4]);
            }
        }
        output
    }

    fn complete(task: &mut OriginalPreviewTask, budget: u64, generation: Generation) {
        let total = task.progress().total();
        let mut previous = 0_u64;
        loop {
            let expected = previous.saturating_add(budget).min(total);
            match task.advance(budget, generation).unwrap() {
                PreviewAdvance::InProgress(progress) => {
                    assert_eq!(progress.completed(), expected);
                    assert!(progress.completed() < total);
                    previous = progress.completed();
                }
                PreviewAdvance::Completed(progress) => {
                    assert_eq!(progress.completed(), total);
                    assert_eq!(progress.completed(), expected);
                    break;
                }
            }
        }
    }

    #[test]
    fn geometry_never_upscales_floors_and_bounds_the_longest_edge() {
        for ((width, height), expected) in [
            ((1600, 1600), (1600, 1600)),
            ((1, 1), (1, 1)),
            ((7, 3), (7, 3)),
            ((1601, 1601), (1600, 1600)),
            ((3200, 1600), (1600, 800)),
            ((1700, 40), (1600, 37)),
            ((30, 2000), (24, 1600)),
            ((4000, 1), (1600, 1)),
            ((1, 4000), (1, 1600)),
            ((3200, 3), (1600, 1)),
            ((1601, 2), (1600, 1)),
        ] {
            assert_eq!(output_dimensions(width, height).unwrap(), expected);
        }
        assert_eq!(output_dimensions(u32::MAX, u32::MAX).unwrap(), (1600, 1600));
        assert_eq!(output_dimensions(u32::MAX, 1).unwrap(), (1600, 1));
        assert_eq!(output_dimensions(1, u32::MAX).unwrap(), (1, 1600));
        assert_eq!(
            output_dimensions(0, 5),
            Err(ContractError::InvalidDimensions)
        );
        assert_eq!(
            output_dimensions(5, 0),
            Err(ContractError::InvalidDimensions)
        );
    }

    #[test]
    fn chunked_output_matches_the_independent_nearest_center_oracle() {
        for (width, height) in [
            (1700, 40),
            (30, 2000),
            (4000, 1),
            (1, 2100),
            (7, 3),
            (1600, 2),
            (1601, 2),
        ] {
            let input = source(width, height);
            let (out_width, out_height) = output_dimensions(width, height).unwrap();
            let expected = oracle(input.pixels(), width, out_width, height, out_height);
            for budget in [1, 17, 4096, u64::MAX] {
                let generation = Generation::new(2);
                let mut task = OriginalPreviewTask::new(Arc::clone(&input), generation).unwrap();
                assert_eq!((task.width(), task.height()), (out_width, out_height));
                assert_eq!(
                    task.progress().total(),
                    u64::from(out_width) * u64::from(out_height)
                );
                complete(&mut task, budget, generation);
                let frame = task.finish(generation).unwrap();
                assert_eq!((frame.width(), frame.height()), (out_width, out_height));
                assert_eq!(
                    frame.pixels(),
                    expected.as_slice(),
                    "{width}x{height} {budget}"
                );
            }
        }
    }

    #[test]
    fn small_sources_are_identical_and_alpha_is_preserved() {
        let mut raw = pattern(7, 3);
        raw[3] = 0;
        raw[7] = 1;
        raw[11] = 254;
        let input = Arc::new(
            DecodedSource::from_rgba(
                Dimensions::new(7, 3).unwrap(),
                raw.clone(),
                DecodeLimits::default(),
            )
            .unwrap(),
        );
        let generation = Generation::new(1);
        let mut task = OriginalPreviewTask::new(input, generation).unwrap();
        complete(&mut task, 5, generation);
        let frame = task.finish(generation).unwrap();
        assert_eq!((frame.width(), frame.height()), (7, 3));
        assert_eq!(frame.into_pixels(), raw);
    }

    #[test]
    fn zero_budget_is_rejected_without_mutation() {
        let generation = Generation::new(4);
        let mut task = OriginalPreviewTask::new(source(40, 30), generation).unwrap();
        assert_eq!(
            task.advance(0, generation),
            Err(ContractError::InvalidProgress)
        );
        assert_eq!(task.progress().completed(), 0);
        assert!(task.pixels.is_empty());
        task.advance(5, generation).unwrap();
        assert_eq!(
            task.advance(0, generation),
            Err(ContractError::InvalidProgress)
        );
        assert_eq!(task.progress().completed(), 5);
        assert_eq!(task.pixels.len(), 20);
        task.cancel();
        assert_eq!(
            task.advance(0, generation),
            Err(ContractError::InvalidProgress)
        );
        assert_eq!(task.advance(1, generation), Err(ContractError::Cancelled));
    }

    #[test]
    fn cancellation_staleness_and_incomplete_finish_are_terminal() {
        let input = source(40, 30);
        let generation = Generation::new(3);

        let mut before = OriginalPreviewTask::new(Arc::clone(&input), generation).unwrap();
        before.cancel();
        assert_eq!(before.advance(1, generation), Err(ContractError::Cancelled));
        assert_eq!(before.pixels.capacity(), 0);
        assert_eq!(before.progress().completed(), 0);
        assert_eq!(before.advance(1, generation), Err(ContractError::Cancelled));
        assert_eq!(before.finish(generation), Err(ContractError::Cancelled));

        let mut during = OriginalPreviewTask::new(Arc::clone(&input), generation).unwrap();
        during.advance(5, generation).unwrap();
        during.cancel();
        assert_eq!(during.advance(1, generation), Err(ContractError::Cancelled));
        assert_eq!(during.pixels.capacity(), 0);
        assert_eq!(during.progress().completed(), 5);

        let mut stale = OriginalPreviewTask::new(Arc::clone(&input), generation).unwrap();
        stale.advance(5, generation).unwrap();
        assert_eq!(
            stale.advance(1, Generation::new(4)),
            Err(ContractError::StaleGeneration)
        );
        assert_eq!(stale.pixels.capacity(), 0);
        assert_eq!(
            stale.advance(1, generation),
            Err(ContractError::StaleGeneration)
        );
        assert_eq!(
            stale.finish(generation),
            Err(ContractError::StaleGeneration)
        );

        let mut after = OriginalPreviewTask::new(Arc::clone(&input), generation).unwrap();
        assert!(matches!(
            after.advance(u64::MAX, generation),
            Ok(PreviewAdvance::Completed(_))
        ));
        after.cancel();
        assert_eq!(after.finish(generation), Err(ContractError::Cancelled));

        let mut late = OriginalPreviewTask::new(Arc::clone(&input), generation).unwrap();
        late.advance(u64::MAX, generation).unwrap();
        assert_eq!(
            late.finish(Generation::new(9)),
            Err(ContractError::StaleGeneration)
        );

        let mut partial = OriginalPreviewTask::new(input, generation).unwrap();
        partial.advance(7, generation).unwrap();
        assert_eq!(
            partial.finish(generation),
            Err(ContractError::InvalidProgress)
        );
    }

    #[test]
    fn task_shares_the_source_without_cloning_and_outlives_the_caller() {
        let input = source(40, 30);
        let pointer = input.pixels().as_ptr();
        let expected = input.pixels().to_vec();
        let generation = Generation::new(1);
        let mut task = OriginalPreviewTask::new(Arc::clone(&input), generation).unwrap();
        assert_eq!(Arc::strong_count(&input), 2);
        assert!(std::ptr::eq(task.source.pixels().as_ptr(), pointer));
        assert!(!std::ptr::eq(task.pixels.as_ptr(), pointer));
        drop(input);
        assert_eq!(Arc::strong_count(&task.source), 1);
        complete(&mut task, 64, generation);
        let frame = task.finish(generation).unwrap();
        assert_eq!(frame.into_pixels(), expected);
    }

    #[test]
    fn output_allocation_is_bounded_by_the_preview_ceiling() {
        let generation = Generation::new(0);
        let ceiling = usize::try_from(MAX_OUTPUT_BYTES).unwrap();
        let largest = OriginalPreviewTask::new(source(1601, 1601), generation).unwrap();
        assert_eq!((largest.width(), largest.height()), (1600, 1600));
        assert_eq!(largest.progress().total(), 2_560_000);
        assert!(largest.pixels.capacity() >= ceiling);
        assert!(largest.pixels.is_empty());
        let tall = OriginalPreviewTask::new(source(30, 2000), generation).unwrap();
        assert!(tall.pixels.capacity() >= 24 * 1600 * 4);
        assert!(tall.pixels.capacity() <= ceiling);
    }
}
