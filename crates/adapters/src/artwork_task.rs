//! Shared lifecycle for generation-bound, row-budgeted artwork export tasks.

use std::fmt;
use std::sync::Arc;

use image_to_binary_core::{Cancellation, ContractError, Generation, Progress};
use image_to_binary_export::{BrowserChunkDrain, ExportError};
use image_to_binary_processing::{RenderError, RenderOptions, Renderer, SampleGrid};

/// Failures returned by the artwork export tasks.
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
            Self::Terminated => formatter.write_str("artwork export task was already terminated"),
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

/// Output and storage ceilings shared by [`crate::ArtworkPngTask`] and
/// [`crate::ArtworkTextTask`].
///
/// `max_encoded_bytes` bounds the whole encoded output, including the PNG header
/// and trailer for PNG. `chunk_size` is the size of each completed chunk and must
/// be in `1..=1 MiB`. `max_pending_bytes` bounds undrained chunk storage and must
/// be at least `chunk_size`. Pending storage is not tied to the row budget, so it
/// must hold everything one `advance` can emit; PNG should also exceed the
/// encoder's internal 4 KiB write granularity. A breach is terminal even if the
/// caller drains between advances.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PngTaskLimits {
    max_encoded_bytes: u64,
    chunk_size: usize,
    max_pending_bytes: u64,
}

impl PngTaskLimits {
    /// Creates limits; the task constructors validate them.
    #[must_use]
    pub const fn new(max_encoded_bytes: u64, chunk_size: usize, max_pending_bytes: u64) -> Self {
        Self {
            max_encoded_bytes,
            chunk_size,
            max_pending_bytes,
        }
    }

    pub(crate) const fn into_parts(self) -> (u64, usize, u64) {
        (
            self.max_encoded_bytes,
            self.chunk_size,
            self.max_pending_bytes,
        )
    }
}

/// Encoder-specific row output and sink ownership plugged into [`ArtworkTask`].
pub trait RowEncoder: Sized {
    /// Name reported by the task's `Debug` output.
    const TASK_NAME: &'static str;
    /// Reason reported when `finish` runs on a task that already finished.
    const FINISHED_REASON: &'static str;

    /// Renders grid rows `start..start + count` and writes them to the sink.
    fn write_rows(
        &mut self,
        renderer: &mut Renderer,
        grid: &SampleGrid,
        options: RenderOptions,
        start: u32,
        count: u32,
    ) -> Result<(), PngTaskError>;

    /// Finalizes the encoder's own stream before the core takes remaining chunks.
    fn finish_rows(self) -> Result<(), PngTaskError>;
}

/// Encoder, sink drain, and batch sizing handed to [`ArtworkTask::start`].
pub struct TaskSetup<E> {
    pub(crate) encoder: E,
    pub(crate) drain: BrowserChunkDrain,
    pub(crate) batch_rows: u32,
    pub(crate) grid_row_units: u64,
}

pub struct ActiveJob<E> {
    grid: Arc<SampleGrid>,
    renderer: Renderer,
    options: RenderOptions,
    encoder: E,
    drain: BrowserChunkDrain,
    next_row: u32,
    pub(crate) batch_rows: u32,
}

impl<E: RowEncoder> ActiveJob<E> {
    fn encode_rows(&mut self, progress: &mut Progress, mut rows: u64) -> Result<(), PngTaskError> {
        while rows > 0 {
            let batch = u32::try_from(rows.min(u64::from(self.batch_rows)))
                .map_err(|_| PngTaskError::Contract(ContractError::ArithmeticOverflow))?;
            self.encoder.write_rows(
                &mut self.renderer,
                &self.grid,
                self.options,
                self.next_row,
                batch,
            )?;
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

pub enum TaskState<E> {
    Active(Box<ActiveJob<E>>),
    Finished,
    Terminated(Option<ContractError>),
}

/// Resumable, generation-bound export of one sampled grid through a row encoder.
///
/// Releasing the task (by dropping it, finishing it, or a terminal failure)
/// releases the grid, renderer, encoder, and chunk storage.
///
/// Each advance renders at most its row budget of grid rows and feeds them to
/// the encoder, whose output lands in browser chunk storage. Only one row batch
/// is alive at a time, and a batch never exceeds the encoder's byte ceiling;
/// larger spans are split internally.
///
/// Invariants: progress counts source-grid rows and never exceeds the grid
/// height. Cancellation, a stale generation, a render failure, an encoding
/// failure, or a sink failure is terminal: the encoder, chunk storage, renderer,
/// and grid are released, and later calls return the same contract error or
/// [`PngTaskError::Terminated`]. Usage errors such as a zero budget or finishing
/// before all rows are encoded leave the task unchanged.
pub struct ArtworkTask<E: RowEncoder> {
    progress: Progress,
    grid_row_units: u64,
    cancellation: Cancellation,
    pub(crate) state: TaskState<E>,
}

impl<E: RowEncoder> fmt::Debug for ArtworkTask<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct(E::TASK_NAME)
            .field("progress", &self.progress)
            .finish_non_exhaustive()
    }
}

impl<E: RowEncoder> ArtworkTask<E> {
    /// Creates an active task whose progress covers every row of `grid`.
    ///
    /// # Errors
    ///
    /// Returns a contract error when the grid height cannot form progress.
    pub(crate) fn start(
        grid: Arc<SampleGrid>,
        renderer: Renderer,
        options: RenderOptions,
        generation: Generation,
        setup: TaskSetup<E>,
    ) -> Result<Self, PngTaskError> {
        let TaskSetup {
            encoder,
            drain,
            batch_rows,
            grid_row_units,
        } = setup;
        let progress = Progress::new(u64::from(grid.height())).map_err(PngTaskError::Contract)?;
        Ok(Self {
            progress,
            grid_row_units,
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

    /// Returns the encoder's per-grid-row unit count, fixed at construction.
    pub(crate) const fn grid_row_units(&self) -> u64 {
        self.grid_row_units
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

    /// Finalizes the encoder exactly once and returns every remaining chunk.
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
            .finish_rows()
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
                E::FINISHED_REASON,
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

pub(crate) fn export_error(error: ExportError) -> PngTaskError {
    match error {
        ExportError::Contract(contract) => PngTaskError::Contract(contract),
        other => PngTaskError::Export(other),
    }
}
