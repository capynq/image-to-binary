use std::fmt;
use std::sync::Arc;

use image_to_binary_core::{Cancellation, ContractError, Generation, Progress};
use image_to_binary_export::{
    browser_chunk_channel, BrowserChunkDrain, BrowserChunkWriter, ExportError, PngExportPlan,
    PngStreamEncoder,
};
use image_to_binary_processing::{
    RenderError, RenderOptions, Renderer, SampleGrid, MAX_RENDER_ROW_BYTES,
};

/// Failures returned by [`ArtworkPngTask`].
#[derive(Debug)]
pub enum PngTaskError {
    /// A contract, cancellation, or generation check rejected the call.
    Contract(ContractError),
    /// Rendering a row batch failed.
    Render(RenderError),
    /// Encoding or browser chunk storage failed.
    Export(ExportError),
    /// A previous cancellation or failure already released the task.
    Terminated,
}

impl fmt::Display for PngTaskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => error.fmt(formatter),
            Self::Render(error) => error.fmt(formatter),
            Self::Export(error) => error.fmt(formatter),
            Self::Terminated => formatter.write_str("PNG export task was already terminated"),
        }
    }
}

impl std::error::Error for PngTaskError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Contract(error) => Some(error),
            Self::Render(error) => Some(error),
            Self::Export(error) => Some(error),
            Self::Terminated => None,
        }
    }
}

struct ActiveJob {
    grid: Arc<SampleGrid>,
    renderer: Renderer,
    options: RenderOptions,
    encoder: PngStreamEncoder<BrowserChunkWriter>,
    drain: BrowserChunkDrain,
    next_row: u32,
    batch_rows: u32,
}

impl ActiveJob {
    fn encode_rows(&mut self, progress: &mut Progress, mut rows: u64) -> Result<(), PngTaskError> {
        while rows > 0 {
            let batch = u32::try_from(rows.min(u64::from(self.batch_rows)))
                .map_err(|_| PngTaskError::Contract(ContractError::ArithmeticOverflow))?;
            let pixels = self
                .renderer
                .render_rows(&self.grid, self.options, self.next_row, batch)
                .map_err(PngTaskError::Render)?;
            self.encoder.write_rows(&pixels).map_err(export_error)?;
            self.next_row = self
                .next_row
                .checked_add(batch)
                .ok_or(PngTaskError::Contract(ContractError::ArithmeticOverflow))?;
            progress
                .advance(u64::from(batch))
                .map_err(PngTaskError::Contract)?;
            rows = rows
                .checked_sub(u64::from(batch))
                .ok_or(PngTaskError::Contract(ContractError::ArithmeticOverflow))?;
        }
        Ok(())
    }
}

enum TaskState {
    Active(Box<ActiveJob>),
    Finished,
    Terminated(Option<ContractError>),
}

/// Output and storage ceilings for an [`ArtworkPngTask`].
///
/// `max_encoded_bytes` bounds the whole PNG, including header and trailer.
/// `chunk_size` is the size of each completed chunk and must be in
/// `1..=1 MiB`. `max_pending_bytes` bounds undrained chunk storage and must be
/// at least `chunk_size`. Pending storage is not tied to the row budget, so it
/// must hold everything one `advance` can emit, and it should exceed png's
/// internal 4 KiB write granularity. A breach is terminal even if the caller
/// drains between advances.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PngTaskLimits {
    max_encoded_bytes: u64,
    chunk_size: usize,
    max_pending_bytes: u64,
}

impl PngTaskLimits {
    /// Creates limits; [`ArtworkPngTask::new`] validates them.
    #[must_use]
    pub const fn new(max_encoded_bytes: u64, chunk_size: usize, max_pending_bytes: u64) -> Self {
        Self {
            max_encoded_bytes,
            chunk_size,
            max_pending_bytes,
        }
    }
}

/// Owned resumable PNG export of one sampled artwork grid.
///
/// Releasing the task (by dropping it, finishing it, or a terminal failure)
/// releases the grid, renderer, encoder, and chunk storage.
///
/// Each advance renders at most its row budget of grid rows, converts them to
/// RGBA8 pixel rows, and feeds them to a streaming PNG encoder whose output
/// lands in browser chunk storage. Only one row batch of pixels is alive at a
/// time, and a batch never exceeds [`MAX_RENDER_ROW_BYTES`] or the render
/// options' output ceiling; larger spans are split internally.
///
/// Invariants: progress counts source-grid rows and never exceeds the grid
/// height. A cancellation, stale generation, rendering failure, encoding
/// failure, or sink failure is terminal: the encoder, the chunk storage,
/// the renderer, and the grid are released, and later calls return the same
/// contract error or [`PngTaskError::Terminated`]. Usage errors such as a zero
/// budget or finishing before all rows are encoded leave the task unchanged.
///
/// Browser Blob accumulation retains encoded bytes, so the browser path is not
/// constant-memory; only the rendered frame is never retained whole. Encoding
/// within one row batch is synchronous and cannot be preempted; cancellation
/// takes effect at the next advance or finish boundary.
pub struct ArtworkPngTask {
    progress: Progress,
    grid_row_pixels: u64,
    cancellation: Cancellation,
    state: TaskState,
}

impl fmt::Debug for ArtworkPngTask {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArtworkPngTask")
            .field("progress", &self.progress)
            .finish_non_exhaustive()
    }
}

impl ArtworkPngTask {
    /// Creates a PNG export task for one generation over `grid`.
    ///
    /// The PNG header and browser chunk storage are reserved synchronously.
    ///
    /// # Errors
    ///
    /// Returns a render error for invalid options or grid geometry, an export
    /// error for invalid output or chunk settings, and
    /// [`PngTaskError::Contract`] with [`ContractError::ResourceLimitExceeded`]
    /// when a single source row cannot fit the row byte ceiling.
    pub fn new(
        grid: Arc<SampleGrid>,
        renderer: Renderer,
        options: RenderOptions,
        generation: Generation,
        limits: PngTaskLimits,
    ) -> Result<Self, PngTaskError> {
        let PngTaskLimits {
            max_encoded_bytes,
            chunk_size,
            max_pending_bytes,
        } = limits;
        let (frame_width, frame_height) = renderer
            .frame_dimensions(&grid, options)
            .map_err(PngTaskError::Render)?;
        let batch_rows = batch_rows(&grid, frame_width, frame_height, options)?;
        let grid_row_pixels = u64::from(frame_width)
            .checked_mul(u64::from(tile_height(&grid, frame_height)?))
            .filter(|pixels| *pixels > 0)
            .ok_or(PngTaskError::Contract(ContractError::ResourceLimitExceeded))?;
        let plan = PngExportPlan::new(frame_width, frame_height, max_encoded_bytes)
            .map_err(export_error)?;
        let (writer, drain) =
            browser_chunk_channel(chunk_size, max_pending_bytes).map_err(export_error)?;
        let encoder = plan.stream_encoder(writer).map_err(export_error)?;
        let progress = Progress::new(u64::from(grid.height())).map_err(PngTaskError::Contract)?;
        Ok(Self {
            progress,
            grid_row_pixels,
            cancellation: Cancellation::new(generation),
            state: TaskState::Active(Box::new(ActiveJob {
                grid,
                renderer,
                options,
                encoder,
                drain,
                next_row: 0,
                batch_rows,
            })),
        })
    }

    /// Returns completed and total source-grid rows.
    #[must_use]
    pub const fn progress(&self) -> Progress {
        self.progress
    }

    /// Returns the output pixels one grid row contributes to the frame
    /// (frame width times tile height). Always at least one, so a caller can
    /// convert a pixel budget into whole rows without dividing by zero.
    #[must_use]
    pub const fn pixels_per_grid_row(&self) -> u64 {
        self.grid_row_pixels
    }

    /// Requests cancellation; the next boundary call releases the task.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    /// Renders and encodes at most `row_budget` grid rows.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidProgress`] for a zero budget without
    /// changing state. Cancellation and stale generations are checked at entry
    /// and exit; a failed check, render, encode, or sink operation makes the
    /// task terminal.
    pub fn advance(
        &mut self,
        generation: Generation,
        row_budget: u64,
    ) -> Result<Progress, PngTaskError> {
        if row_budget == 0 {
            return Err(PngTaskError::Contract(ContractError::InvalidProgress));
        }
        self.ensure_open()?;
        if let Err(error) = self.cancellation.check(generation) {
            return Err(self.fail(PngTaskError::Contract(error)));
        }
        let remaining = self
            .progress
            .total()
            .checked_sub(self.progress.completed())
            .ok_or(PngTaskError::Contract(ContractError::InvalidProgress))?;
        let count = row_budget.min(remaining);
        let result = match &mut self.state {
            TaskState::Active(job) => job.encode_rows(&mut self.progress, count),
            _ => Err(PngTaskError::Terminated),
        };
        let result = result.and_then(|()| {
            self.cancellation
                .check(generation)
                .map_err(PngTaskError::Contract)
        });
        match result {
            Ok(()) => Ok(self.progress),
            Err(error) => Err(self.fail(error)),
        }
    }

    /// Moves completed chunks out of storage; each chunk is returned once.
    ///
    /// # Errors
    ///
    /// Returns a terminal error after cancellation, after a poisoned chunk lock,
    /// or when the task is finished or already terminated.
    pub fn drain_chunks(&mut self) -> Result<Vec<Vec<u8>>, PngTaskError> {
        self.ensure_open()?;
        if self.cancellation.is_cancelled() {
            return Err(self.fail(PngTaskError::Contract(ContractError::Cancelled)));
        }
        let result = match &self.state {
            TaskState::Active(job) => job.drain.drain().map_err(export_error),
            _ => Err(PngTaskError::Terminated),
        };
        result.map_err(|error| self.fail(error))
    }

    /// Finalizes the PNG exactly once and returns every remaining chunk.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidProgress`] when rows remain, without
    /// changing state. A second finish returns [`ContractError::InvalidSettings`].
    /// Cancellation, stale generations, and encoder or sink failures are
    /// terminal.
    pub fn finish(&mut self, generation: Generation) -> Result<Vec<Vec<u8>>, PngTaskError> {
        self.ensure_open()?;
        if let Err(error) = self.cancellation.check(generation) {
            return Err(self.fail(PngTaskError::Contract(error)));
        }
        if self.progress.completed() != self.progress.total() {
            return Err(PngTaskError::Contract(ContractError::InvalidProgress));
        }
        let job = match std::mem::replace(&mut self.state, TaskState::Finished) {
            TaskState::Active(job) => *job,
            other => {
                self.state = other;
                return Err(PngTaskError::Terminated);
            }
        };
        let ActiveJob { encoder, drain, .. } = job;
        let result = encoder
            .finish()
            .map_err(export_error)
            .and_then(|()| drain.take_remaining().map_err(export_error));
        match result {
            Ok(chunks) => Ok(chunks),
            Err(error) => Err(self.fail(error)),
        }
    }

    fn ensure_open(&self) -> Result<(), PngTaskError> {
        match &self.state {
            TaskState::Active(_) => Ok(()),
            TaskState::Finished => Err(PngTaskError::Contract(ContractError::InvalidSettings(
                "finished PNG job",
            ))),
            TaskState::Terminated(Some(error)) => Err(PngTaskError::Contract(error.clone())),
            TaskState::Terminated(None) => Err(PngTaskError::Terminated),
        }
    }

    fn fail(&mut self, error: PngTaskError) -> PngTaskError {
        let retained = match &error {
            PngTaskError::Contract(contract) => Some(contract.clone()),
            PngTaskError::Render(_) | PngTaskError::Export(_) | PngTaskError::Terminated => None,
        };
        self.state = TaskState::Terminated(retained);
        error
    }
}

fn export_error(error: ExportError) -> PngTaskError {
    match error {
        ExportError::Contract(contract) => PngTaskError::Contract(contract),
        other => PngTaskError::Export(other),
    }
}

fn tile_height(grid: &SampleGrid, frame_height: u32) -> Result<u32, PngTaskError> {
    frame_height
        .checked_div(grid.height())
        .ok_or(PngTaskError::Contract(ContractError::InvalidDimensions))
}

fn batch_rows(
    grid: &SampleGrid,
    frame_width: u32,
    frame_height: u32,
    options: RenderOptions,
) -> Result<u32, PngTaskError> {
    let overflow = || PngTaskError::Contract(ContractError::ArithmeticOverflow);
    let tile_height = tile_height(grid, frame_height)?;
    let row_group_bytes = u64::from(frame_width)
        .checked_mul(4)
        .and_then(|bytes| bytes.checked_mul(u64::from(tile_height)))
        .ok_or_else(overflow)?;
    let budget = MAX_RENDER_ROW_BYTES.min(options.max_output_bytes);
    let rows = budget
        .checked_div(row_group_bytes)
        .filter(|rows| *rows > 0)
        .ok_or(PngTaskError::Contract(ContractError::ResourceLimitExceeded))?;
    u32::try_from(rows.min(u64::from(grid.height()))).map_err(|_| overflow())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image_to_binary_core::Sampling;
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

    fn options(sampling: Sampling, font_size: u8) -> RenderOptions {
        RenderOptions {
            cell_size: 5,
            font_size,
            sampling,
            ..RenderOptions::default()
        }
    }

    fn new_task(
        grid: &Arc<SampleGrid>,
        options: RenderOptions,
        max_encoded_bytes: u64,
        max_pending_bytes: u64,
    ) -> ArtworkPngTask {
        ArtworkPngTask::new(
            Arc::clone(grid),
            Renderer::new().unwrap(),
            options,
            GENERATION,
            PngTaskLimits::new(max_encoded_bytes, CHUNK_BYTES, max_pending_bytes),
        )
        .unwrap()
    }

    fn default_task(grid: &Arc<SampleGrid>, options: RenderOptions) -> ArtworkPngTask {
        new_task(grid, options, u64::MAX, PENDING_BYTES)
    }

    fn decode_pixels(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes.to_vec()))
            .read_info()
            .unwrap();
        let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut decoded).unwrap();
        (info.width, info.height, decoded)
    }

    fn run_to_completion(task: &mut ArtworkPngTask, budget: u64) -> Vec<u8> {
        let mut bytes = Vec::new();
        while task.progress().completed() < task.progress().total() {
            let before = task.progress().completed();
            let progress = task.advance(GENERATION, budget).unwrap();
            let delta = progress.completed() - before;
            assert!(delta > 0 && delta <= budget);
            for chunk in task.drain_chunks().unwrap() {
                assert_eq!(chunk.len(), CHUNK_BYTES);
                bytes.extend(chunk);
            }
        }
        for chunk in task.finish(GENERATION).unwrap() {
            bytes.extend(chunk);
        }
        bytes
    }

    #[test]
    fn task_is_send() {
        assert_send::<ArtworkPngTask>();
    }

    #[test]
    fn incremental_pixels_match_full_frame_and_plan_bytes() {
        for (sampling, font_size) in [(Sampling::Classic, 8), (Sampling::Average, 30)] {
            let grid = sampled(85, 61, sampling, font_size);
            let render_options = options(sampling, font_size);
            let full = Renderer::new()
                .unwrap()
                .render(&grid, render_options)
                .unwrap();
            let row_length = usize::try_from(u64::from(full.width()) * 4).unwrap();
            let mut expected_bytes = Vec::new();
            PngExportPlan::new(full.width(), full.height(), u64::MAX)
                .unwrap()
                .encode(
                    &mut expected_bytes,
                    &Cancellation::new(GENERATION),
                    GENERATION,
                    |row| {
                        let start = usize::try_from(row).unwrap() * row_length;
                        Ok(full.pixels()[start..start + row_length].to_vec())
                    },
                )
                .unwrap();
            for budget in [1, 2, 3, u64::from(grid.height()) + 5] {
                let mut task = default_task(&grid, render_options);
                let bytes = run_to_completion(&mut task, budget);
                assert_eq!(bytes, expected_bytes);
                let (width, height, pixels) = decode_pixels(&bytes);
                assert_eq!((width, height), (full.width(), full.height()));
                assert_eq!(pixels, full.pixels());
            }
        }
    }

    #[test]
    fn advance_obeys_budget_and_rejects_zero() {
        let grid = sampled(85, 61, Sampling::Classic, 8);
        let mut task = default_task(&grid, options(Sampling::Classic, 8));
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
        let row_group = 85 * 4 * 5;
        let split = RenderOptions {
            max_output_bytes: row_group * 2,
            ..options(Sampling::Classic, 8)
        };
        let full = Renderer::new()
            .unwrap()
            .render(&grid, options(Sampling::Classic, 8))
            .unwrap();
        let mut task = new_task(&grid, split, u64::MAX, PENDING_BYTES);
        match &task.state {
            TaskState::Active(job) => {
                assert_eq!(job.batch_rows, 2);
                assert!(job.batch_rows < grid.height());
            }
            _ => panic!("task must start active"),
        }
        let bytes = run_to_completion(&mut task, u64::MAX);
        assert_eq!(decode_pixels(&bytes).2, full.pixels());

        let too_small = RenderOptions {
            max_output_bytes: row_group - 1,
            ..options(Sampling::Classic, 8)
        };
        assert!(matches!(
            ArtworkPngTask::new(
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
    fn dropping_an_unfinished_task_releases_the_grid() {
        let grid = sampled(85, 61, Sampling::Classic, 8);
        let mut task = default_task(&grid, options(Sampling::Classic, 8));
        task.advance(GENERATION, 5).unwrap();
        drop(task);
        assert_eq!(Arc::strong_count(&grid), 1);
    }

    #[test]
    fn finish_releases_the_grid() {
        let grid = sampled(85, 61, Sampling::Classic, 8);
        let mut task = default_task(&grid, options(Sampling::Classic, 8));
        task.advance(GENERATION, u64::MAX).unwrap();
        task.finish(GENERATION).unwrap();
        assert_eq!(Arc::strong_count(&grid), 1);
    }

    #[test]
    fn drained_chunks_move_out_once_and_finish_returns_the_tail() {
        let grid = sampled(400, 400, Sampling::Classic, 8);
        let render_options = options(Sampling::Classic, 8);
        let mut task = default_task(&grid, render_options);
        task.advance(GENERATION, 40).unwrap();
        let mid = task.drain_chunks().unwrap();
        assert!(!mid.is_empty());
        assert!(task.drain_chunks().unwrap().is_empty());
        let mut collected: Vec<u8> = mid.into_iter().flatten().collect();
        task.advance(GENERATION, u64::MAX).unwrap();
        let tail = task.finish(GENERATION).unwrap();
        collected.extend(tail.into_iter().flatten());
        let full = Renderer::new()
            .unwrap()
            .render(&grid, render_options)
            .unwrap();
        assert_eq!(decode_pixels(&collected).2, full.pixels());
    }

    #[test]
    fn finalization_rejects_early_and_double_finish_without_terminating_early_calls() {
        let grid = sampled(85, 61, Sampling::Classic, 8);
        let mut task = default_task(&grid, options(Sampling::Classic, 8));
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
        let mut task = default_task(&grid, options(Sampling::Classic, 8));
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
        let mut task = default_task(&grid, options(Sampling::Classic, 8));
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
        let mut task = new_task(&grid, options(Sampling::Classic, 8), 40, PENDING_BYTES);
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
    }

    #[test]
    fn failed_sink_is_terminal_and_replays_as_terminated() {
        let grid = sampled(85, 61, Sampling::Classic, 8);
        let mut task = new_task(&grid, options(Sampling::Classic, 8), u64::MAX, 64);
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
