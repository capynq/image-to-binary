//! Resumable animated GIF export. Each frame is rendered from its own sampled grid, encoded as one
//! full-canvas GIF frame, and released before the next frame's grid is accepted.

use std::fmt;
use std::sync::Arc;

use image_to_binary_core::{Cancellation, ContractError, Generation, Progress};
use image_to_binary_export::{
    browser_chunk_channel, BrowserChunkDrain, BrowserChunkWriter, GifColors, GifExportPlan,
    GifRepeat, GifStreamEncoder,
};
use image_to_binary_processing::{
    ColorMode, LoopCount, RenderOptions, Renderer, Rgb, SampleGrid, MAX_RENDER_ROW_BYTES,
};

use crate::artwork_task::{export_error, PngTaskError, PngTaskLimits};

/// Resumable GIF export of an animation, one frame per sampled grid.
///
/// The caller supplies each frame's grid with [`ArtworkGifTask::begin_frame`] when
/// [`ArtworkGifTask::needs_grid`] is true, then advances it. Progress counts encoded frames.
/// Frame `n` uses delay `n` from the constructor's table.
///
/// Invariants: every frame has the same canvas size, fixed by the first frame and within the GIF
/// screen limit; a frame's RGBA pixels are capped at `min(MAX_RENDER_ROW_BYTES, max_output_bytes)`,
/// so at most one frame's pixels are alive; the encoder is created with the first frame's header.
/// Encoded output is streamed through browser chunk storage, so one frame's compressed bytes must
/// fit the pending limit, because chunks are drained between frames, not within one.
///
/// Failure contract: cancellation, a stale generation, a render, encode or sink failure is
/// terminal and releases the grid, renderer, encoder and chunk storage. Usage errors such as a
/// zero budget, a frame that is not awaited, or finishing early leave the task unchanged.
pub struct ArtworkGifTask {
    progress: Progress,
    cancellation: Cancellation,
    state: GifState,
}

enum GifState {
    Active(Box<ActiveGif>),
    Finished,
    Terminated(Option<ContractError>),
}

struct ActiveGif {
    renderer: Renderer,
    options: RenderOptions,
    delays: Vec<u16>,
    plan: GifPlanSettings,
    canvas: Option<(u32, u32)>,
    writer: Option<BrowserChunkWriter>,
    encoder: Option<GifStreamEncoder<BrowserChunkWriter>>,
    drain: BrowserChunkDrain,
    frame: Option<FrameRender>,
}

#[derive(Clone, Copy)]
struct GifPlanSettings {
    max_encoded_bytes: u64,
    repeat: GifRepeat,
    colors: GifColors,
}

struct FrameRender {
    grid: Arc<SampleGrid>,
    next_row: u32,
    pixels: Vec<u8>,
    pixels_per_row: u64,
    batch_rows: u32,
}

impl ArtworkGifTask {
    /// Creates a GIF export of `delays.len()` frames, each shown for its delay in centiseconds.
    ///
    /// The colour policy follows `options.style`: monochrome uses the two style colours as one
    /// global palette, and source colour quantises each frame.
    ///
    /// # Errors
    ///
    /// Returns a contract error for an empty frame table or invalid chunk settings, and
    /// [`PngTaskError::Export`] when browser chunk storage cannot be created.
    pub fn new(
        renderer: Renderer,
        options: RenderOptions,
        generation: Generation,
        delays: Vec<u16>,
        loop_count: LoopCount,
        limits: PngTaskLimits,
    ) -> Result<Self, PngTaskError> {
        let (max_encoded_bytes, chunk_size, max_pending_bytes) = limits.into_parts();
        let total = u64::try_from(delays.len())
            .map_err(|_| PngTaskError::Contract(ContractError::ArithmeticOverflow))?;
        let progress = Progress::new(total).map_err(PngTaskError::Contract)?;
        let (writer, drain) =
            browser_chunk_channel(chunk_size, max_pending_bytes).map_err(export_error)?;
        let repeat = match loop_count {
            LoopCount::Infinite => GifRepeat::Infinite,
            LoopCount::Finite(count) => GifRepeat::Finite(count),
        };
        let colors = match options.style.color {
            ColorMode::Monochrome => GifColors::Monochrome {
                background: rgb(options.style.background),
                foreground: rgb(options.style.foreground),
            },
            ColorMode::Source => GifColors::Source,
        };
        Ok(Self {
            progress,
            cancellation: Cancellation::new(generation),
            state: GifState::Active(Box::new(ActiveGif {
                renderer,
                options,
                delays,
                plan: GifPlanSettings {
                    max_encoded_bytes,
                    repeat,
                    colors,
                },
                canvas: None,
                writer: Some(writer),
                encoder: None,
                drain,
                frame: None,
            })),
        })
    }

    /// Returns encoded and total frames.
    #[must_use]
    pub const fn progress(&self) -> Progress {
        self.progress
    }

    /// Returns the index of the frame the next grid belongs to.
    #[must_use]
    pub fn frame_index(&self) -> usize {
        usize::try_from(self.progress.completed()).unwrap_or(usize::MAX)
    }

    /// Returns true when the task is waiting for the grid of [`Self::frame_index`].
    #[must_use]
    pub fn needs_grid(&self) -> bool {
        match &self.state {
            GifState::Active(active) => {
                active.frame.is_none() && self.progress.completed() < self.progress.total()
            }
            _ => false,
        }
    }

    /// Requests cancellation; the next boundary call releases the task.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    /// Starts rendering the awaited frame from `grid`. The grid must belong to that frame.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidProgress`] without changing state when no frame is
    /// awaited. A canvas that differs from the first frame, a canvas over the render ceiling, or a
    /// render, encoder or sink failure is terminal.
    pub fn begin_frame(
        &mut self,
        generation: Generation,
        grid: Arc<SampleGrid>,
    ) -> Result<(), PngTaskError> {
        self.ensure_open()?;
        if !self.needs_grid() {
            return Err(PngTaskError::Contract(ContractError::InvalidProgress));
        }
        if let Err(error) = self.cancellation.check(generation) {
            return Err(self.fail(PngTaskError::Contract(error)));
        }
        let result = match &mut self.state {
            GifState::Active(active) => {
                active.begin(grid, self.progress.completed(), self.progress.total())
            }
            _ => Err(PngTaskError::Terminated),
        };
        result.map_err(|error| self.fail(error))
    }

    /// Renders at most `pixel_budget` output pixels of the awaited frame and encodes the frame
    /// once every row is rendered. The budget converts to whole grid rows of at least one row.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidProgress`] for a zero budget or when no frame is being
    /// rendered, without changing state. Cancellation, a stale generation, a render failure, or an
    /// encoder or sink failure is terminal.
    pub fn advance(
        &mut self,
        generation: Generation,
        pixel_budget: u64,
    ) -> Result<Progress, PngTaskError> {
        if pixel_budget == 0 {
            return Err(PngTaskError::Contract(ContractError::InvalidProgress));
        }
        self.ensure_open()?;
        let rendering = matches!(
            &self.state,
            GifState::Active(active) if active.frame.is_some()
        );
        if !rendering {
            return Err(PngTaskError::Contract(ContractError::InvalidProgress));
        }
        if let Err(error) = self.cancellation.check(generation) {
            return Err(self.fail(PngTaskError::Contract(error)));
        }
        let result = match &mut self.state {
            GifState::Active(active) => active.render(
                generation,
                &self.cancellation,
                pixel_budget,
                &mut self.progress,
            ),
            _ => Err(PngTaskError::Terminated),
        };
        match result {
            Ok(()) => Ok(self.progress),
            Err(error) => Err(self.fail(error)),
        }
    }

    /// Moves completed chunks out of storage; each chunk is returned once.
    ///
    /// # Errors
    ///
    /// Returns a terminal error after cancellation, a poisoned chunk lock, or when the task is
    /// finished or already terminated.
    pub fn drain_chunks(&mut self) -> Result<Vec<Vec<u8>>, PngTaskError> {
        self.ensure_open()?;
        if self.cancellation.is_cancelled() {
            return Err(self.fail(PngTaskError::Contract(ContractError::Cancelled)));
        }
        let result = match &self.state {
            GifState::Active(active) => active.drain.drain().map_err(export_error),
            _ => Err(PngTaskError::Terminated),
        };
        result.map_err(|error| self.fail(error))
    }

    /// Writes the GIF trailer once every frame is encoded and returns every remaining chunk.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidProgress`] while frames remain, without changing state. A
    /// second finish returns [`ContractError::InvalidSettings`]. Cancellation, a stale generation,
    /// and encoder or sink failures are terminal.
    pub fn finish(&mut self, generation: Generation) -> Result<Vec<Vec<u8>>, PngTaskError> {
        self.ensure_open()?;
        if let Err(error) = self.cancellation.check(generation) {
            return Err(self.fail(PngTaskError::Contract(error)));
        }
        if self.progress.completed() != self.progress.total() {
            return Err(PngTaskError::Contract(ContractError::InvalidProgress));
        }
        let active = match std::mem::replace(&mut self.state, GifState::Finished) {
            GifState::Active(active) => *active,
            other => {
                self.state = other;
                return Err(PngTaskError::Terminated);
            }
        };
        let ActiveGif { encoder, drain, .. } = active;
        let result = encoder
            .ok_or(PngTaskError::Terminated)
            .and_then(|encoder| encoder.finish().map_err(export_error))
            .and_then(|()| drain.take_remaining().map_err(export_error));
        match result {
            Ok(chunks) => Ok(chunks),
            Err(error) => Err(self.fail(error)),
        }
    }

    fn ensure_open(&self) -> Result<(), PngTaskError> {
        match &self.state {
            GifState::Active(_) => Ok(()),
            GifState::Finished => Err(PngTaskError::Contract(ContractError::InvalidSettings(
                "finished GIF export",
            ))),
            GifState::Terminated(Some(error)) => Err(PngTaskError::Contract(error.clone())),
            GifState::Terminated(None) => Err(PngTaskError::Terminated),
        }
    }

    fn fail(&mut self, error: PngTaskError) -> PngTaskError {
        let retained = match &error {
            PngTaskError::Contract(contract) => Some(contract.clone()),
            PngTaskError::Render(_) | PngTaskError::Export(_) | PngTaskError::Terminated => None,
        };
        self.state = GifState::Terminated(retained);
        error
    }
}

impl ActiveGif {
    fn begin(&mut self, grid: Arc<SampleGrid>, index: u64, total: u64) -> Result<(), PngTaskError> {
        if index >= total {
            return Err(PngTaskError::Contract(ContractError::InvalidProgress));
        }
        let (width, height) = self
            .renderer
            .frame_dimensions(&grid, self.options)
            .map_err(PngTaskError::Render)?;
        let row_bytes = u64::from(width)
            .checked_mul(4)
            .ok_or(PngTaskError::Contract(ContractError::ArithmeticOverflow))?;
        let frame_bytes = row_bytes
            .checked_mul(u64::from(height))
            .ok_or(PngTaskError::Contract(ContractError::ArithmeticOverflow))?;
        let frame_limit = MAX_RENDER_ROW_BYTES.min(self.options.max_output_bytes);
        if frame_bytes > frame_limit {
            return Err(PngTaskError::Contract(ContractError::ResourceLimitExceeded));
        }
        let tile_height = height
            .checked_div(grid.height())
            .filter(|rows| *rows > 0)
            .ok_or(PngTaskError::Contract(ContractError::InvalidDimensions))?;
        let group_bytes = row_bytes
            .checked_mul(u64::from(tile_height))
            .filter(|bytes| *bytes > 0)
            .ok_or(PngTaskError::Contract(ContractError::ResourceLimitExceeded))?;
        let batch_rows = u32::try_from((frame_limit / group_bytes).min(u64::from(grid.height())))
            .map_err(|_| PngTaskError::Contract(ContractError::ArithmeticOverflow))?;
        if batch_rows == 0 {
            return Err(PngTaskError::Contract(ContractError::ResourceLimitExceeded));
        }
        match self.canvas {
            None => {
                let plan = GifExportPlan::new(
                    width,
                    height,
                    self.plan.max_encoded_bytes,
                    self.plan.repeat,
                    self.plan.colors,
                )
                .map_err(export_error)?;
                let writer = self
                    .writer
                    .take()
                    .ok_or(PngTaskError::Contract(ContractError::InvalidProgress))?;
                self.encoder = Some(plan.stream_encoder(writer).map_err(export_error)?);
                self.canvas = Some((width, height));
            }
            Some(canvas) if canvas == (width, height) => {}
            Some(_) => return Err(PngTaskError::Contract(ContractError::InvalidDimensions)),
        }
        let frame_len = usize::try_from(frame_bytes)
            .map_err(|_| PngTaskError::Contract(ContractError::ArithmeticOverflow))?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(frame_len)
            .map_err(|_| PngTaskError::Contract(ContractError::ResourceLimitExceeded))?;
        self.frame = Some(FrameRender {
            grid,
            next_row: 0,
            pixels,
            pixels_per_row: group_bytes / 4,
            batch_rows,
        });
        Ok(())
    }

    fn render(
        &mut self,
        generation: Generation,
        cancellation: &Cancellation,
        pixel_budget: u64,
        progress: &mut Progress,
    ) -> Result<(), PngTaskError> {
        let Some(frame) = self.frame.as_mut() else {
            return Err(PngTaskError::Contract(ContractError::InvalidProgress));
        };
        let height = frame.grid.height();
        let remaining = u64::from(
            height
                .checked_sub(frame.next_row)
                .ok_or(PngTaskError::Contract(ContractError::InvalidProgress))?,
        );
        let budget_rows = (pixel_budget / frame.pixels_per_row).max(1);
        let mut rows = budget_rows.min(remaining);
        while rows > 0 {
            let batch = u32::try_from(rows.min(u64::from(frame.batch_rows)))
                .map_err(|_| PngTaskError::Contract(ContractError::ArithmeticOverflow))?;
            let bytes = self
                .renderer
                .render_rows(&frame.grid, self.options, frame.next_row, batch)
                .map_err(PngTaskError::Render)?;
            frame.pixels.extend_from_slice(&bytes);
            frame.next_row = frame
                .next_row
                .checked_add(batch)
                .ok_or(PngTaskError::Contract(ContractError::ArithmeticOverflow))?;
            rows = rows
                .checked_sub(u64::from(batch))
                .ok_or(PngTaskError::Contract(ContractError::ArithmeticOverflow))?;
            cancellation
                .check(generation)
                .map_err(PngTaskError::Contract)?;
        }
        if frame.next_row == height {
            self.encode_frame(generation, cancellation, progress)?;
        }
        Ok(())
    }

    fn encode_frame(
        &mut self,
        generation: Generation,
        cancellation: &Cancellation,
        progress: &mut Progress,
    ) -> Result<(), PngTaskError> {
        cancellation
            .check(generation)
            .map_err(PngTaskError::Contract)?;
        let index = usize::try_from(progress.completed())
            .map_err(|_| PngTaskError::Contract(ContractError::ArithmeticOverflow))?;
        let delay = *self
            .delays
            .get(index)
            .ok_or(PngTaskError::Contract(ContractError::InvalidProgress))?;
        let mut frame = self
            .frame
            .take()
            .ok_or(PngTaskError::Contract(ContractError::InvalidProgress))?;
        let encoder = self.encoder.as_mut().ok_or(PngTaskError::Terminated)?;
        encoder
            .write_frame(&mut frame.pixels, delay)
            .map_err(export_error)?;
        progress.advance(1).map_err(PngTaskError::Contract)?;
        Ok(())
    }
}

fn rgb(color: Rgb) -> [u8; 3] {
    [color.red, color.green, color.blue]
}

impl fmt::Debug for ArtworkGifTask {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArtworkGifTask")
            .field("progress", &self.progress)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image_to_binary_core::Sampling;
    use image_to_binary_processing::{decode, DecodeLimits, Sampler};

    const GENERATION: Generation = Generation::new(9);
    const PENDING_BYTES: u64 = 1 << 20;
    const CHUNK_BYTES: usize = 64;

    fn sampled(width: u32, height: u32) -> Arc<SampleGrid> {
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
        let source = decode(&bytes, DecodeLimits::default()).unwrap();
        Sampler::new(Arc::new(source))
            .sample(5, Sampling::Classic)
            .unwrap()
    }

    fn options() -> RenderOptions {
        RenderOptions {
            cell_size: 5,
            font_size: 8,
            ..RenderOptions::default()
        }
    }

    fn task(delays: Vec<u16>, limits: PngTaskLimits) -> ArtworkGifTask {
        ArtworkGifTask::new(
            Renderer::new().unwrap(),
            options(),
            GENERATION,
            delays,
            LoopCount::Infinite,
            limits,
        )
        .unwrap()
    }

    fn limits() -> PngTaskLimits {
        PngTaskLimits::new(u64::MAX, CHUNK_BYTES, PENDING_BYTES)
    }

    fn run(
        task: &mut ArtworkGifTask,
        grids: &[Arc<SampleGrid>],
        budget: u64,
    ) -> Result<Vec<u8>, PngTaskError> {
        let mut bytes = Vec::new();
        for grid in grids {
            task.begin_frame(GENERATION, Arc::clone(grid))?;
            let target = task.frame_index() + 1;
            while task.frame_index() < target {
                task.advance(GENERATION, budget)?;
                for chunk in task.drain_chunks()? {
                    bytes.extend(chunk);
                }
            }
        }
        for chunk in task.finish(GENERATION)? {
            bytes.extend(chunk);
        }
        Ok(bytes)
    }

    #[test]
    fn task_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<ArtworkGifTask>();
    }

    #[test]
    fn output_does_not_depend_on_the_pixel_budget() {
        let grids = [sampled(40, 30), sampled(40, 30), sampled(40, 30)];
        let whole = run(&mut task(vec![5, 10, 50], limits()), &grids, u64::MAX).unwrap();
        assert!(whole.starts_with(b"GIF89a"));
        for budget in [1, 7, 500] {
            let stepped = run(&mut task(vec![5, 10, 50], limits()), &grids, budget).unwrap();
            assert_eq!(stepped, whole);
        }
    }

    #[test]
    fn a_usage_error_leaves_the_task_unchanged() {
        let grid = sampled(40, 30);
        let mut task = task(vec![10], limits());
        assert!(task.needs_grid());
        assert!(matches!(
            task.advance(GENERATION, 1),
            Err(PngTaskError::Contract(ContractError::InvalidProgress))
        ));
        assert!(matches!(
            task.finish(GENERATION),
            Err(PngTaskError::Contract(ContractError::InvalidProgress))
        ));
        task.begin_frame(GENERATION, Arc::clone(&grid)).unwrap();
        assert!(matches!(
            task.begin_frame(GENERATION, Arc::clone(&grid)),
            Err(PngTaskError::Contract(ContractError::InvalidProgress))
        ));
        assert!(matches!(
            task.advance(GENERATION, 0),
            Err(PngTaskError::Contract(ContractError::InvalidProgress))
        ));
        assert!(task.advance(GENERATION, 1).is_ok());
    }

    #[test]
    fn cancellation_releases_the_awaited_grid_and_is_terminal() {
        let grid = sampled(40, 30);
        let mut task = task(vec![10, 10], limits());
        task.begin_frame(GENERATION, Arc::clone(&grid)).unwrap();
        assert_eq!(Arc::strong_count(&grid), 2);
        task.advance(GENERATION, 3).unwrap();
        task.cancel();
        assert!(matches!(
            task.advance(GENERATION, 1),
            Err(PngTaskError::Contract(ContractError::Cancelled))
        ));
        assert_eq!(Arc::strong_count(&grid), 1);
        assert!(matches!(
            task.drain_chunks(),
            Err(PngTaskError::Contract(ContractError::Cancelled))
        ));
    }

    #[test]
    fn the_output_ceiling_is_a_typed_terminal_error() {
        let grid = sampled(40, 30);
        let mut task = task(vec![10], PngTaskLimits::new(64, CHUNK_BYTES, PENDING_BYTES));
        let mut failure = task.begin_frame(GENERATION, Arc::clone(&grid)).err();
        while failure.is_none() && task.frame_index() < 1 {
            failure = task.advance(GENERATION, u64::MAX).err();
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
    fn frames_must_share_one_canvas() {
        let mut task = task(vec![10, 10], limits());
        task.begin_frame(GENERATION, sampled(40, 30)).unwrap();
        while task.frame_index() < 1 {
            task.advance(GENERATION, u64::MAX).unwrap();
        }
        assert!(matches!(
            task.begin_frame(GENERATION, sampled(45, 30)),
            Err(PngTaskError::Contract(ContractError::InvalidDimensions))
        ));
    }
}
