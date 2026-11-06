use std::io::{self, Write};
use std::sync::Arc;

use image_to_binary_core::{ContractError, Generation};
use image_to_binary_export::{browser_chunk_channel, BrowserChunkWriter, ExportError};
use image_to_binary_processing::{
    RenderOptions, Renderer, SampleGrid, MAX_TEXT_GLYPH_BYTES, MAX_TEXT_ROW_BYTES,
};

use crate::artwork_task::{
    export_error, ArtworkTask, PngTaskError, PngTaskLimits, RowEncoder, TaskSetup,
};

/// Row encoder that writes UTF-8 text lines into browser chunk storage.
pub struct TextRows {
    writer: BrowserChunkWriter,
    emitted: u64,
    max_encoded_bytes: u64,
}

impl RowEncoder for TextRows {
    const TASK_NAME: &'static str = "ArtworkTextTask";
    const FINISHED_REASON: &'static str = "finished text job";

    fn write_rows(
        &mut self,
        renderer: &mut Renderer,
        grid: &SampleGrid,
        options: RenderOptions,
        start: u32,
        count: u32,
    ) -> Result<(), PngTaskError> {
        let text = renderer
            .text_rows(grid, options, start, count)
            .map_err(PngTaskError::Render)?;
        let length = u64::try_from(text.len())
            .map_err(|_| PngTaskError::Contract(ContractError::ArithmeticOverflow))?;
        let emitted = self
            .emitted
            .checked_add(length)
            .ok_or(PngTaskError::Contract(ContractError::ArithmeticOverflow))?;
        if emitted > self.max_encoded_bytes {
            return Err(PngTaskError::Contract(ContractError::ResourceLimitExceeded));
        }
        self.writer
            .write_all(&text)
            .map_err(|error| io_error(&error))?;
        self.emitted = emitted;
        Ok(())
    }

    fn finish_rows(self) -> Result<(), PngTaskError> {
        Ok(())
    }
}

fn io_error(error: &io::Error) -> PngTaskError {
    match error
        .get_ref()
        .and_then(|source| source.downcast_ref::<ContractError>())
    {
        Some(contract) => PngTaskError::Contract(contract.clone()),
        None => PngTaskError::Export(ExportError::Sink("pending chunk storage lock was poisoned")),
    }
}

/// Owned resumable UTF-8 text export of one sampled grid.
///
/// Output follows the text format defined on [`Renderer::text_rows`], so the
/// concatenated chunks are byte-identical to a single full-grid text render.
/// Lifecycle, budget, cancellation, and terminal rules are those of
/// [`crate::ArtworkPngTask`], and both share one core.
///
/// `limits.max_pending_bytes` must hold everything one `advance` can emit,
/// because pending storage is not tied to the row budget. A breach is terminal
/// even if the caller drains between advances.
///
/// Peak memory is one text batch of at most `min(MAX_TEXT_ROW_BYTES,
/// max_output_bytes)` bytes, the undrained chunks bounded by `max_pending_bytes`,
/// and the grid. Browser Blob accumulation retains every encoded byte, so the
/// browser path is not constant-memory.
pub type ArtworkTextTask = ArtworkTask<TextRows>;

impl ArtworkTask<TextRows> {
    /// Creates a text export task for one generation over `grid`.
    ///
    /// The chunk storage is reserved synchronously; no text is rendered yet.
    ///
    /// # Errors
    ///
    /// Returns a render error for invalid options or grid geometry,
    /// [`ContractError::InvalidSettings`] for a zero encoded ceiling, an export
    /// error for invalid chunk settings, and [`ContractError::ResourceLimitExceeded`]
    /// when one grid row cannot fit the text batch ceiling.
    pub fn new(
        grid: Arc<SampleGrid>,
        renderer: Renderer,
        options: RenderOptions,
        generation: Generation,
        limits: PngTaskLimits,
    ) -> Result<Self, PngTaskError> {
        let (max_encoded_bytes, chunk_size, max_pending_bytes) = limits.into_parts();
        if max_encoded_bytes == 0 {
            return Err(PngTaskError::Contract(ContractError::InvalidSettings(
                "output limit",
            )));
        }
        renderer
            .frame_dimensions(&grid, options)
            .map_err(PngTaskError::Render)?;
        let row_bytes = text_row_bound(&grid)?;
        let batch_rows = text_batch_rows(&grid, row_bytes, options)?;
        let (writer, drain) =
            browser_chunk_channel(chunk_size, max_pending_bytes).map_err(export_error)?;
        Self::start(
            grid,
            renderer,
            options,
            generation,
            TaskSetup {
                encoder: TextRows {
                    writer,
                    emitted: 0,
                    max_encoded_bytes,
                },
                drain,
                batch_rows,
                grid_row_units: row_bytes,
            },
        )
    }
}

fn text_row_bound(grid: &SampleGrid) -> Result<u64, PngTaskError> {
    let overflow = || PngTaskError::Contract(ContractError::ArithmeticOverflow);
    let glyph_bytes = u64::try_from(MAX_TEXT_GLYPH_BYTES).map_err(|_| overflow())?;
    u64::from(grid.width())
        .checked_mul(glyph_bytes)
        .and_then(|bytes| bytes.checked_add(1))
        .ok_or_else(overflow)
}

fn text_batch_rows(
    grid: &SampleGrid,
    row_bytes: u64,
    options: RenderOptions,
) -> Result<u32, PngTaskError> {
    let budget = MAX_TEXT_ROW_BYTES.min(options.max_output_bytes);
    let rows = budget
        .checked_div(row_bytes)
        .filter(|rows| *rows > 0)
        .ok_or(PngTaskError::Contract(ContractError::ResourceLimitExceeded))?;
    u32::try_from(rows.min(u64::from(grid.height())))
        .map_err(|_| PngTaskError::Contract(ContractError::ArithmeticOverflow))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artwork_task::TaskState;
    use image_to_binary_core::{Ramp, RenderMode, Sampling};
    use image_to_binary_processing::{decode, DecodeLimits, Sampler};

    const CHUNK_BYTES: usize = 64;
    const PENDING_BYTES: u64 = 1 << 20;
    const GENERATION: Generation = Generation::new(5);

    fn assert_send<T: Send>() {}

    fn source_png(width: u32, height: u32) -> Vec<u8> {
        let mut pixels = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let value: u8 = if (x / 5 + y / 5) % 2 == 0 { 0 } else { 255 };
                pixels.extend_from_slice(&[value, value, value, 255]);
            }
        }
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        bytes
    }

    fn sampled(width: u32, height: u32, sampling: Sampling, font_size: u8) -> Arc<SampleGrid> {
        let source = decode(&source_png(width, height), DecodeLimits::default()).unwrap();
        let mut sampler = Sampler::new(Arc::new(source));
        match sampling {
            Sampling::Classic => sampler.sample(5, Sampling::Classic).unwrap(),
            Sampling::Average => {
                let geometry = Renderer::new()
                    .unwrap()
                    .aspect_geometry(5, font_size)
                    .unwrap();
                sampler
                    .sample_average_rect(5, u32::from(geometry.source_cell_height))
                    .unwrap()
            }
        }
    }

    fn options(sampling: Sampling, font_size: u8, mode: RenderMode) -> RenderOptions {
        RenderOptions {
            cell_size: 5,
            font_size,
            sampling,
            mode,
            ..RenderOptions::default()
        }
    }

    fn new_task_with_chunk(
        grid: &Arc<SampleGrid>,
        options: RenderOptions,
        chunk: usize,
        max_encoded_bytes: u64,
        max_pending_bytes: u64,
    ) -> ArtworkTextTask {
        ArtworkTextTask::new(
            Arc::clone(grid),
            Renderer::new().unwrap(),
            options,
            GENERATION,
            PngTaskLimits::new(max_encoded_bytes, chunk, max_pending_bytes),
        )
        .unwrap()
    }

    fn new_task(
        grid: &Arc<SampleGrid>,
        options: RenderOptions,
        max_encoded_bytes: u64,
        max_pending_bytes: u64,
    ) -> ArtworkTextTask {
        new_task_with_chunk(
            grid,
            options,
            CHUNK_BYTES,
            max_encoded_bytes,
            max_pending_bytes,
        )
    }

    fn default_task(grid: &Arc<SampleGrid>, options: RenderOptions) -> ArtworkTextTask {
        new_task(grid, options, u64::MAX, PENDING_BYTES)
    }

    fn full_text(grid: &Arc<SampleGrid>, options: RenderOptions) -> Vec<u8> {
        Renderer::new()
            .unwrap()
            .text_rows(grid, options, 0, grid.height())
            .unwrap()
    }

    fn run_to_completion(task: &mut ArtworkTextTask, budget: u64, chunk: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        while task.progress().completed() < task.progress().total() {
            let before = task.progress().completed();
            let progress = task.advance(GENERATION, budget).unwrap();
            let delta = progress.completed() - before;
            assert!(delta > 0 && delta <= budget);
            for drained in task.drain_chunks().unwrap() {
                assert_eq!(drained.len(), chunk);
                bytes.extend(drained);
            }
        }
        for tail in task.finish(GENERATION).unwrap() {
            bytes.extend(tail);
        }
        bytes
    }

    #[test]
    fn task_is_send() {
        assert_send::<ArtworkTextTask>();
    }

    #[test]
    fn binary_output_matches_exact_bytes() {
        let grid = sampled(15, 10, Sampling::Classic, 8);
        let mut task = default_task(&grid, options(Sampling::Classic, 8, RenderMode::Binary));
        assert_eq!(
            run_to_completion(&mut task, u64::MAX, CHUNK_BYTES),
            b"101\n010\n"
        );
    }

    #[test]
    fn chunks_concatenate_to_full_text_across_budgets() {
        let grids = [
            (
                sampled(85, 61, Sampling::Classic, 8),
                options(Sampling::Classic, 8, RenderMode::Binary),
            ),
            (
                sampled(85, 61, Sampling::Average, 30),
                options(Sampling::Average, 30, RenderMode::Block),
            ),
            (
                sampled(85, 61, Sampling::Average, 30),
                options(
                    Sampling::Average,
                    30,
                    RenderMode::Ascii(Ramp::DEFAULT_ASCII),
                ),
            ),
        ];
        for (grid, render_options) in grids {
            let expected = full_text(&grid, render_options);
            for budget in [1, 2, 3, u64::from(grid.height()) + 5] {
                let mut task = default_task(&grid, render_options);
                assert_eq!(
                    run_to_completion(&mut task, budget, CHUNK_BYTES),
                    expected,
                    "budget {budget}"
                );
            }
        }
    }

    #[test]
    fn single_byte_chunks_reassemble_multibyte_block_characters() {
        let grid = sampled(85, 61, Sampling::Average, 30);
        let render_options = options(Sampling::Average, 30, RenderMode::Block);
        let expected = full_text(&grid, render_options);
        assert!(expected.iter().any(|byte| *byte >= 0x80));
        let mut task = new_task_with_chunk(&grid, render_options, 1, u64::MAX, PENDING_BYTES);
        assert_eq!(run_to_completion(&mut task, 3, 1), expected);
    }

    #[test]
    fn advance_obeys_budget_and_rejects_zero() {
        let grid = sampled(85, 61, Sampling::Classic, 8);
        let mut task = default_task(&grid, options(Sampling::Classic, 8, RenderMode::Binary));
        let total = task.progress().total();
        assert!(matches!(
            task.advance(GENERATION, 0),
            Err(PngTaskError::Contract(ContractError::InvalidProgress))
        ));
        assert_eq!(task.progress().completed(), 0);
        let progress = task.advance(GENERATION, 3).unwrap();
        assert_eq!(progress.completed(), 3);
        let progress = task.advance(GENERATION, u64::MAX).unwrap();
        assert_eq!(progress.completed(), total);
        assert!(task.finish(GENERATION).is_ok());
    }

    #[test]
    fn oversized_batches_are_split_within_the_output_ceiling() {
        let grid = sampled(85, 61, Sampling::Classic, 8);
        let row_bytes = 17 * 3 + 1;
        let split = RenderOptions {
            max_output_bytes: row_bytes * 2,
            ..options(Sampling::Classic, 8, RenderMode::Binary)
        };
        let mut task = new_task(&grid, split, u64::MAX, PENDING_BYTES);
        match &task.state {
            TaskState::Active(job) => {
                assert_eq!(job.batch_rows, 2);
                assert!(job.batch_rows < grid.height());
            }
            _ => panic!("task must start active"),
        }
        let expected = full_text(&grid, options(Sampling::Classic, 8, RenderMode::Binary));
        assert_eq!(
            run_to_completion(&mut task, u64::MAX, CHUNK_BYTES),
            expected
        );

        let too_small = RenderOptions {
            max_output_bytes: row_bytes - 1,
            ..options(Sampling::Classic, 8, RenderMode::Binary)
        };
        assert!(matches!(
            ArtworkTextTask::new(
                Arc::clone(&grid),
                Renderer::new().unwrap(),
                too_small,
                GENERATION,
                PngTaskLimits::new(u64::MAX, CHUNK_BYTES, PENDING_BYTES),
            ),
            Err(PngTaskError::Contract(ContractError::ResourceLimitExceeded))
        ));
    }

    #[test]
    fn zero_encoded_ceiling_and_invalid_options_are_rejected_at_creation() {
        let grid = sampled(85, 61, Sampling::Classic, 8);
        let render_options = options(Sampling::Classic, 8, RenderMode::Binary);
        assert!(matches!(
            ArtworkTextTask::new(
                Arc::clone(&grid),
                Renderer::new().unwrap(),
                render_options,
                GENERATION,
                PngTaskLimits::new(0, CHUNK_BYTES, PENDING_BYTES),
            ),
            Err(PngTaskError::Contract(ContractError::InvalidSettings(_)))
        ));
        let invalid = options(Sampling::Average, 8, RenderMode::Block);
        assert!(matches!(
            ArtworkTextTask::new(
                grid,
                Renderer::new().unwrap(),
                invalid,
                GENERATION,
                PngTaskLimits::new(u64::MAX, CHUNK_BYTES, PENDING_BYTES),
            ),
            Err(PngTaskError::Render(_))
        ));
    }

    #[test]
    fn dropping_an_unfinished_task_releases_the_grid() {
        let grid = sampled(85, 61, Sampling::Classic, 8);
        let mut task = default_task(&grid, options(Sampling::Classic, 8, RenderMode::Binary));
        task.advance(GENERATION, 5).unwrap();
        drop(task);
        assert_eq!(Arc::strong_count(&grid), 1);
    }

    #[test]
    fn finish_releases_the_grid() {
        let grid = sampled(85, 61, Sampling::Classic, 8);
        let mut task = default_task(&grid, options(Sampling::Classic, 8, RenderMode::Binary));
        task.advance(GENERATION, u64::MAX).unwrap();
        task.finish(GENERATION).unwrap();
        assert_eq!(Arc::strong_count(&grid), 1);
    }

    #[test]
    fn drained_chunks_move_out_once_and_finish_returns_the_tail() {
        let grid = sampled(400, 400, Sampling::Average, 30);
        let render_options = options(Sampling::Average, 30, RenderMode::Block);
        let mut task = default_task(&grid, render_options);
        task.advance(GENERATION, 40).unwrap();
        let mid = task.drain_chunks().unwrap();
        assert!(!mid.is_empty());
        assert!(task.drain_chunks().unwrap().is_empty());
        let mut collected: Vec<u8> = mid.into_iter().flatten().collect();
        task.advance(GENERATION, u64::MAX).unwrap();
        let tail = task.finish(GENERATION).unwrap();
        collected.extend(tail.into_iter().flatten());
        assert_eq!(collected, full_text(&grid, render_options));
    }

    #[test]
    fn finalization_rejects_early_and_double_finish_without_terminating_early_calls() {
        let grid = sampled(85, 61, Sampling::Classic, 8);
        let mut task = default_task(&grid, options(Sampling::Classic, 8, RenderMode::Binary));
        task.advance(GENERATION, 1).unwrap();
        assert!(matches!(
            task.finish(GENERATION),
            Err(PngTaskError::Contract(ContractError::InvalidProgress))
        ));
        task.advance(GENERATION, u64::MAX).unwrap();
        task.finish(GENERATION).unwrap();
        assert!(matches!(
            task.finish(GENERATION),
            Err(PngTaskError::Contract(ContractError::InvalidSettings(_)))
        ));
        assert!(task.advance(GENERATION, 1).is_err());
        assert!(task.drain_chunks().is_err());
    }

    #[test]
    fn cancellation_discards_state_and_releases_resources() {
        let grid = sampled(85, 61, Sampling::Classic, 8);
        let mut task = default_task(&grid, options(Sampling::Classic, 8, RenderMode::Binary));
        assert_eq!(Arc::strong_count(&grid), 2);
        task.advance(GENERATION, 4).unwrap();
        task.cancel();
        assert!(matches!(
            task.advance(GENERATION, 1),
            Err(PngTaskError::Contract(ContractError::Cancelled))
        ));
        assert_eq!(Arc::strong_count(&grid), 1);
        assert!(matches!(
            task.advance(GENERATION, 1),
            Err(PngTaskError::Contract(ContractError::Cancelled))
        ));
        assert!(matches!(
            task.drain_chunks(),
            Err(PngTaskError::Contract(ContractError::Cancelled))
        ));
        assert!(matches!(
            task.finish(GENERATION),
            Err(PngTaskError::Contract(ContractError::Cancelled))
        ));
    }

    #[test]
    fn stale_generation_is_terminal_at_the_boundary() {
        let grid = sampled(85, 61, Sampling::Classic, 8);
        let mut task = default_task(&grid, options(Sampling::Classic, 8, RenderMode::Binary));
        task.advance(GENERATION, 2).unwrap();
        assert!(matches!(
            task.advance(Generation::new(4), 1),
            Err(PngTaskError::Contract(ContractError::StaleGeneration))
        ));
        assert!(matches!(
            task.advance(GENERATION, 1),
            Err(PngTaskError::Contract(ContractError::StaleGeneration))
        ));
        assert!(matches!(
            task.finish(GENERATION),
            Err(PngTaskError::Contract(ContractError::StaleGeneration))
        ));
    }

    #[test]
    fn encoded_limit_exceeded_is_terminal() {
        let grid = sampled(85, 61, Sampling::Classic, 8);
        let mut task = new_task(
            &grid,
            options(Sampling::Classic, 8, RenderMode::Binary),
            40,
            PENDING_BYTES,
        );
        let mut failure = None;
        while task.progress().completed() < task.progress().total() {
            if let Err(error) = task.advance(GENERATION, u64::MAX) {
                failure = Some(error);
                break;
            }
        }
        if failure.is_none() {
            failure = task.finish(GENERATION).err();
        }
        assert!(matches!(
            failure,
            Some(PngTaskError::Contract(ContractError::ResourceLimitExceeded))
        ));
        assert!(matches!(
            task.advance(GENERATION, 1),
            Err(PngTaskError::Contract(ContractError::ResourceLimitExceeded))
        ));
        assert!(task.finish(GENERATION).is_err());
        assert_eq!(Arc::strong_count(&grid), 1);
    }

    #[test]
    fn pending_limit_breach_is_terminal_and_replays_the_contract_error() {
        let grid = sampled(85, 61, Sampling::Classic, 8);
        let mut task = new_task(
            &grid,
            options(Sampling::Classic, 8, RenderMode::Binary),
            u64::MAX,
            64,
        );
        let mut failure = None;
        while task.progress().completed() < task.progress().total() {
            if let Err(error) = task.advance(GENERATION, u64::MAX) {
                failure = Some(error);
                break;
            }
        }
        if failure.is_none() {
            failure = task.finish(GENERATION).err();
        }
        assert!(matches!(
            failure,
            Some(PngTaskError::Contract(ContractError::ResourceLimitExceeded))
        ));
        assert!(matches!(
            task.advance(GENERATION, 1),
            Err(PngTaskError::Contract(ContractError::ResourceLimitExceeded))
        ));
        assert!(matches!(
            task.drain_chunks(),
            Err(PngTaskError::Contract(ContractError::ResourceLimitExceeded))
        ));
        assert!(matches!(
            task.finish(GENERATION),
            Err(PngTaskError::Contract(ContractError::ResourceLimitExceeded))
        ));
        assert_eq!(Arc::strong_count(&grid), 1);
    }
}
