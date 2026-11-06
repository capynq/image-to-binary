//! Pure incremental preview session; the JS glue in `lib.rs` only converts types.
//!
//! Invariants: one source, one sample cache, one renderer, at most one job. A
//! job generation is accepted only when strictly newer than every earlier
//! accepted one; calls naming any other generation are stale.

use std::mem;
use std::sync::Arc;

use image_to_binary_adapters::{ArtworkPngTask, PngTaskLimits};
use image_to_binary_core::{
    ContractError, Dimensions, Generation, ResourceLimits, Sampling, SourceMetadata,
};
use image_to_binary_processing::{
    decode, DecodeLimits, DecodedSource, OriginalPreviewTask, PreviewAdvance, PreviewTask,
    RenderOptions, RenderedFrame, Renderer, SampleGrid, Sampler, SamplingAdvance, SamplingPlan,
    SamplingTask,
};

use crate::error::{BindingError, ErrorCode};
use crate::numbers::{positive, ranged, safe_integer};

const MAX_ENCODED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_DECODED_BYTES: u64 = 1024 * 1024 * 1024;

/// Ceiling for one browser-exported PNG, header and trailer included. It equals
/// the encoded input ceiling so an export never produces more bytes than the
/// session already accepts as input.
const EXPORT_MAX_ENCODED_BYTES: u64 = MAX_ENCODED_BYTES;

/// Size of each completed chunk copied into a JS-owned `Uint8Array`. One
/// mebibyte bounds every boundary copy while keeping a maximal export to at most
/// 256 arrays.
const EXPORT_CHUNK_BYTES: usize = 1024 * 1024;

/// Pending chunk storage ceiling. It equals the encoded ceiling because the
/// browser retains every encoded byte until the Blob is assembled, so one
/// advance emitting the whole output must not fail merely because the caller has
/// not drained yet.
const EXPORT_MAX_PENDING_BYTES: u64 = EXPORT_MAX_ENCODED_BYTES;

const EXPORT_LIMITS: PngTaskLimits = PngTaskLimits::new(
    EXPORT_MAX_ENCODED_BYTES,
    EXPORT_CHUNK_BYTES,
    EXPORT_MAX_PENDING_BYTES,
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Stage {
    Idle,
    Sampling,
    Preview,
    Encoding,
    Ready,
    Failed,
}

/// What a sampled grid is turned into; both kinds share one job slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Output {
    Preview,
    Export,
}

fn disposed() -> BindingError {
    BindingError::new(ErrorCode::Disposed, "the session has been disposed")
}

fn no_job() -> BindingError {
    BindingError::new(
        ErrorCode::NoJob,
        "there is no active job for this generation",
    )
}

fn stale(message: &str) -> BindingError {
    BindingError::new(ErrorCode::StaleGeneration, message)
}

fn other_kind(expected: Output) -> BindingError {
    let message = match expected {
        Output::Preview => "there is no active preview for this generation",
        Output::Export => "there is no active export for this generation",
    };
    BindingError::new(ErrorCode::NoJob, message)
}

fn changed() -> BindingError {
    BindingError::new(ErrorCode::Internal, "job state changed during finish")
}

/// Validated source geometry; checks everything before any pixel is copied.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SourcePlan {
    dimensions: Dimensions,
    limits: DecodeLimits,
    len: usize,
}

impl SourcePlan {
    pub(crate) fn new(
        width: f64,
        height: f64,
        max_pixels: Option<f64>,
    ) -> Result<Self, BindingError> {
        let width = safe_integer(width, "width")?;
        let height = safe_integer(height, "height")?;
        let limits = match max_pixels {
            None => DecodeLimits::default(),
            Some(ceiling) => {
                let ceiling = safe_integer(ceiling, "maxPixels")?;
                let resource =
                    ResourceLimits::new(ceiling, ResourceLimits::default().max_output_bytes())?;
                DecodeLimits::new(resource, MAX_ENCODED_BYTES, MAX_DECODED_BYTES)?
            }
        };
        let too_large = || {
            BindingError::new(
                ErrorCode::ResourceLimitExceeded,
                "image dimension exceeds the pixel ceiling",
            )
        };
        let dimensions = Dimensions::new(
            u32::try_from(width).map_err(|_| too_large())?,
            u32::try_from(height).map_err(|_| too_large())?,
        )?;
        let len = limits.rgba_len(dimensions)?;
        Ok(Self {
            dimensions,
            limits,
            len,
        })
    }

    pub(crate) fn check_len(&self, actual: usize) -> Result<(), BindingError> {
        if actual == self.len {
            Ok(())
        } else {
            Err(BindingError::new(
                ErrorCode::InvalidDimensions,
                "pixels must contain exactly width * height * 4 bytes",
            ))
        }
    }

    pub(crate) fn allocate(&self) -> Result<Vec<u8>, BindingError> {
        let mut buffer = Vec::new();
        buffer.try_reserve_exact(self.len).map_err(|_| {
            BindingError::new(
                ErrorCode::ResourceLimitExceeded,
                "could not reserve memory for the source pixels",
            )
        })?;
        buffer.resize(self.len, 0);
        Ok(buffer)
    }

    pub(crate) fn into_session(self, pixels: Vec<u8>) -> Result<Session, BindingError> {
        Session::new(DecodedSource::from_rgba(
            self.dimensions,
            pixels,
            self.limits,
        )?)
    }
}

fn reserve_zeroed(len: usize) -> Result<Vec<u8>, BindingError> {
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(len).map_err(|_| {
        BindingError::new(
            ErrorCode::ReservationFailed,
            "could not reserve memory for the encoded input",
        )
    })?;
    buffer.resize(len, 0);
    Ok(buffer)
}

/// Validated encoded-input length; the 256 MiB ceiling is checked before any
/// byte is copied into WASM memory.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EncodedPlan {
    len: usize,
}

impl EncodedPlan {
    pub(crate) fn new(len: usize) -> Result<Self, BindingError> {
        if u64::try_from(len).unwrap_or(u64::MAX) > MAX_ENCODED_BYTES {
            return Err(BindingError::new(
                ErrorCode::EncodedTooLarge,
                "encoded input exceeds the 256 MiB limit",
            ));
        }
        Ok(Self { len })
    }

    /// Reserves the zeroed destination fallibly; failure is a coded error, not a trap.
    pub(crate) fn allocate(self) -> Result<Vec<u8>, BindingError> {
        reserve_zeroed(self.len)
    }

    /// Decodes once with the default limits and moves the normalized pixels into
    /// a session without cloning them. The encoded copy is dropped before the
    /// session is built. Decoder allocations are best effort.
    pub(crate) fn decode_session(encoded: Vec<u8>) -> Result<Session, BindingError> {
        let source = decode(&encoded, DecodeLimits::default())?;
        drop(encoded);
        Session::new(source)
    }
}

#[derive(Debug)]
pub(crate) struct Frame {
    width: u32,
    height: u32,
    pixels: Option<Vec<u8>>,
}

impl Frame {
    fn new(frame: RenderedFrame) -> Self {
        Self {
            width: frame.width(),
            height: frame.height(),
            pixels: Some(frame.into_pixels()),
        }
    }

    pub(crate) const fn width(&self) -> u32 {
        self.width
    }

    pub(crate) const fn height(&self) -> u32 {
        self.height
    }

    pub(crate) fn take_pixels(&mut self) -> Result<Vec<u8>, BindingError> {
        self.pixels.take().ok_or_else(|| {
            BindingError::new(
                ErrorCode::FrameConsumed,
                "the frame pixels were already taken",
            )
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct PreviewSettings {
    cell_size: u8,
    font_size: u8,
    threshold: u8,
    sampling: Sampling,
}

impl PreviewSettings {
    fn parse(
        cell_size: f64,
        font_size: f64,
        threshold: f64,
        sampling: Option<&str>,
    ) -> Result<Self, BindingError> {
        let sampling = match sampling {
            Some("classic") => Sampling::Classic,
            Some("average") => Sampling::Average,
            Some(_) => {
                return Err(BindingError::new(
                    ErrorCode::InvalidSettings,
                    "sampling must be \"classic\" or \"average\"",
                ))
            }
            None => {
                return Err(BindingError::new(
                    ErrorCode::InvalidArgument,
                    "sampling must be a string",
                ))
            }
        };
        Ok(Self {
            cell_size: ranged(cell_size, "cellSize", 5, 30)?,
            font_size: ranged(font_size, "fontSize", 8, 30)?,
            threshold: ranged(threshold, "threshold", 0, 255)?,
            sampling,
        })
    }
}

#[derive(Debug)]
enum Phase {
    Sampling(Box<SamplingTask>, RenderOptions, Output),
    Preview(Box<PreviewTask>),
    Ready(Box<PreviewTask>),
    Original(Box<OriginalPreviewTask>),
    OriginalReady(Box<OriginalPreviewTask>),
    Encoding(Box<ArtworkPngTask>),
    ExportReady(Box<ArtworkPngTask>),
    Failed(BindingError),
}

impl Phase {
    fn stage(&self) -> Stage {
        match self {
            Self::Sampling(..) => Stage::Sampling,
            Self::Preview(_) | Self::Original(_) => Stage::Preview,
            Self::Encoding(_) => Stage::Encoding,
            Self::Ready(_) | Self::OriginalReady(_) | Self::ExportReady(_) => Stage::Ready,
            Self::Failed(_) => Stage::Failed,
        }
    }

    fn progress(&self) -> (u64, u64) {
        match self {
            Self::Sampling(task, ..) => (task.progress().completed(), task.progress().total()),
            Self::Preview(task) | Self::Ready(task) => {
                (task.progress().completed(), task.progress().total())
            }
            Self::Original(task) | Self::OriginalReady(task) => {
                (task.progress().completed(), task.progress().total())
            }
            Self::Encoding(task) | Self::ExportReady(task) => {
                (task.progress().completed(), task.progress().total())
            }
            Self::Failed(_) => (0, 0),
        }
    }
}

#[derive(Debug)]
struct Job {
    generation: Generation,
    output: Output,
    phase: Phase,
}

/// Export gets its own renderer so the PNG path never shares preview glyph state.
fn export_phase(
    grid: Arc<SampleGrid>,
    options: RenderOptions,
    generation: Generation,
) -> Result<Phase, BindingError> {
    let task = ArtworkPngTask::new(grid, Renderer::new()?, options, generation, EXPORT_LIMITS)?;
    Ok(Phase::Encoding(Box::new(task)))
}

fn sampled_phase(
    grid: Arc<SampleGrid>,
    renderer: &mut Renderer,
    options: RenderOptions,
    output: Output,
    generation: Generation,
) -> Result<Phase, BindingError> {
    match output {
        Output::Preview => renderer
            .preview_task(grid, options, generation)
            .map(|preview| Phase::Preview(Box::new(preview)))
            .map_err(BindingError::from),
        Output::Export => export_phase(grid, options, generation),
    }
}

fn step_sampling(
    mut task: Box<SamplingTask>,
    sampler: &mut Sampler,
    renderer: &mut Renderer,
    (options, output): (RenderOptions, Output),
    generation: Generation,
    budget: u64,
) -> Phase {
    match task.advance(budget, generation) {
        Ok(SamplingAdvance::InProgress(_)) => Phase::Sampling(task, options, output),
        Ok(SamplingAdvance::Completed(_)) => {
            let outcome = sampler
                .complete(*task, generation)
                .map_err(BindingError::from)
                .and_then(|grid| sampled_phase(grid, renderer, options, output, generation));
            match outcome {
                Ok(phase) => phase,
                Err(error) => Phase::Failed(error),
            }
        }
        Err(error) => Phase::Failed(error.into()),
    }
}

fn step_preview(mut task: Box<PreviewTask>, generation: Generation, budget: u64) -> Phase {
    match task.advance(budget, generation) {
        Ok(PreviewAdvance::InProgress(_)) => Phase::Preview(task),
        Ok(PreviewAdvance::Completed(_)) => Phase::Ready(task),
        Err(error) => Phase::Failed(error.into()),
    }
}

fn step_original(mut task: Box<OriginalPreviewTask>, generation: Generation, budget: u64) -> Phase {
    match task.advance(budget, generation) {
        Ok(PreviewAdvance::InProgress(_)) => Phase::Original(task),
        Ok(PreviewAdvance::Completed(_)) => Phase::OriginalReady(task),
        Err(error) => Phase::Failed(error.into()),
    }
}

/// Converts the pixel budget into whole grid rows, at least one row, then encodes
/// them. The call that completes the last row only marks the export ready; the
/// PNG trailer is written by `finish`.
fn step_encoding(mut task: Box<ArtworkPngTask>, generation: Generation, budget: u64) -> Phase {
    let rows = budget
        .checked_div(task.pixels_per_grid_row())
        .unwrap_or(0)
        .max(1);
    match task.advance(generation, rows) {
        Ok(progress) if progress.completed() == progress.total() => Phase::ExportReady(task),
        Ok(_) => Phase::Encoding(task),
        Err(error) => Phase::Failed(error.into()),
    }
}

impl Job {
    fn advance(
        &mut self,
        sampler: &mut Sampler,
        renderer: &mut Renderer,
        budget: u64,
    ) -> Result<Stage, BindingError> {
        let generation = self.generation;
        let phase = mem::replace(
            &mut self.phase,
            Phase::Failed(BindingError::new(
                ErrorCode::Internal,
                "the job was interrupted",
            )),
        );
        self.phase = match phase {
            Phase::Sampling(task, options, output) => step_sampling(
                task,
                sampler,
                renderer,
                (options, output),
                generation,
                budget,
            ),
            Phase::Preview(task) => step_preview(task, generation, budget),
            Phase::Original(task) => step_original(task, generation, budget),
            Phase::Encoding(task) => step_encoding(task, generation, budget),
            done @ (Phase::Ready(_)
            | Phase::OriginalReady(_)
            | Phase::ExportReady(_)
            | Phase::Failed(_)) => done,
        };
        match &self.phase {
            Phase::Failed(error) => Err(error.clone()),
            phase => Ok(phase.stage()),
        }
    }
}

#[derive(Debug)]
struct Live {
    metadata: SourceMetadata,
    source: Arc<DecodedSource>,
    sampler: Sampler,
    renderer: Renderer,
    job: Option<Job>,
    newest: Option<u64>,
}

impl Live {
    fn check_generation(&self, generation: u64) -> Result<(), BindingError> {
        match self.newest {
            Some(newest) if newest == generation => Ok(()),
            Some(_) => Err(stale("generation does not match the current job")),
            None => Err(no_job()),
        }
    }

    fn start(
        &mut self,
        generation: u64,
        settings: PreviewSettings,
        output: Output,
    ) -> Result<Stage, BindingError> {
        self.ensure_newer(generation)?;
        let job = self.begin(Generation::new(generation), settings, output)?;
        Ok(self.commit(generation, job))
    }

    fn start_original(&mut self, generation: u64) -> Result<Stage, BindingError> {
        self.ensure_newer(generation)?;
        let id = Generation::new(generation);
        let task = OriginalPreviewTask::new(Arc::clone(&self.source), id)?;
        let job = Job {
            generation: id,
            output: Output::Preview,
            phase: Phase::Original(Box::new(task)),
        };
        Ok(self.commit(generation, job))
    }

    fn ensure_newer(&self, generation: u64) -> Result<(), BindingError> {
        if self.newest.is_some_and(|newest| generation <= newest) {
            return Err(stale("generation is not newer than the current job"));
        }
        Ok(())
    }

    fn commit(&mut self, generation: u64, job: Job) -> Stage {
        let stage = job.phase.stage();
        self.newest = Some(generation);
        self.job = Some(job);
        stage
    }

    fn begin(
        &mut self,
        generation: Generation,
        settings: PreviewSettings,
        output: Output,
    ) -> Result<Job, BindingError> {
        let cell_height = match settings.sampling {
            Sampling::Classic => u32::from(settings.cell_size),
            Sampling::Average => u32::from(
                self.renderer
                    .aspect_geometry(settings.cell_size, settings.font_size)?
                    .source_cell_height,
            ),
        };
        let options = RenderOptions {
            cell_size: settings.cell_size,
            font_size: settings.font_size,
            threshold: settings.threshold,
            sampling: settings.sampling,
            ..RenderOptions::default()
        };
        let plan = self.sampler.prepare(
            settings.cell_size,
            cell_height,
            settings.sampling,
            generation,
        )?;
        let phase = match (plan, output) {
            (SamplingPlan::Cached(grid), Output::Preview) => Phase::Preview(Box::new(
                self.renderer.preview_task(grid, options, generation)?,
            )),
            (SamplingPlan::Cached(grid), Output::Export) => {
                export_phase(grid, options, generation)?
            }
            (SamplingPlan::Task(task), output) => Phase::Sampling(task, options, output),
        };
        Ok(Job {
            generation,
            output,
            phase,
        })
    }

    fn advance(&mut self, generation: u64, budget: u64) -> Result<Stage, BindingError> {
        self.check_generation(generation)?;
        let Self {
            sampler,
            renderer,
            job,
            ..
        } = self;
        job.as_mut()
            .ok_or_else(no_job)?
            .advance(sampler, renderer, budget)
    }

    fn cancel(&mut self, generation: u64) -> Result<(), BindingError> {
        self.check_generation(generation)?;
        if let Some(job) = self.job.as_mut() {
            if !matches!(job.phase, Phase::Failed(_)) {
                job.phase = Phase::Failed(BindingError::from(ContractError::Cancelled));
            }
        }
        Ok(())
    }

    fn finish(&mut self, generation: u64) -> Result<Frame, BindingError> {
        self.check_generation(generation)?;
        let job = self.job.as_ref().ok_or_else(no_job)?;
        if job.output != Output::Preview {
            return Err(other_kind(Output::Preview));
        }
        match &job.phase {
            Phase::Ready(_) | Phase::OriginalReady(_) => {}
            Phase::Failed(error) => return Err(error.clone()),
            Phase::Sampling(..) | Phase::Preview(_) | Phase::Original(_) => {
                return Err(BindingError::new(
                    ErrorCode::JobIncomplete,
                    "the job has not finished rendering",
                ))
            }
            Phase::Encoding(_) | Phase::ExportReady(_) => {
                return Err(changed());
            }
        }
        let Some(Job {
            generation: job_generation,
            phase,
            ..
        }) = self.job.take()
        else {
            return Err(changed());
        };
        match phase {
            Phase::Ready(task) => Ok(Frame::new((*task).finish(job_generation)?)),
            Phase::OriginalReady(task) => Ok(Frame::new((*task).finish(job_generation)?)),
            _ => Err(changed()),
        }
    }

    /// Moves completed chunks out of the export; a sampling export has none yet.
    /// A terminal export error is stored as the job's failure.
    fn drain_export(&mut self, generation: u64) -> Result<Vec<Vec<u8>>, BindingError> {
        self.check_generation(generation)?;
        let job = self.job.as_mut().ok_or_else(no_job)?;
        if job.output != Output::Export {
            return Err(other_kind(Output::Export));
        }
        let drained = match &mut job.phase {
            Phase::Sampling(..) => return Ok(Vec::new()),
            Phase::Encoding(task) | Phase::ExportReady(task) => {
                task.drain_chunks().map_err(BindingError::from)
            }
            Phase::Failed(error) => return Err(error.clone()),
            _ => return Err(changed()),
        };
        match drained {
            Ok(chunks) => Ok(chunks),
            Err(error) => {
                job.phase = Phase::Failed(error.clone());
                Err(error)
            }
        }
    }

    /// Finalizes the PNG once and returns every chunk not yet drained. A job that
    /// is already `Failed` (from `advance` or `cancel`) is not released: the call
    /// returns that failure and the job stays in the slot until a newer start
    /// replaces it. A finalization error after the job was taken releases it.
    fn finish_export(&mut self, generation: u64) -> Result<Vec<Vec<u8>>, BindingError> {
        self.check_generation(generation)?;
        let job = self.job.as_ref().ok_or_else(no_job)?;
        if job.output != Output::Export {
            return Err(other_kind(Output::Export));
        }
        match &job.phase {
            Phase::ExportReady(_) => {}
            Phase::Failed(error) => return Err(error.clone()),
            Phase::Sampling(..) | Phase::Encoding(_) => {
                return Err(BindingError::new(
                    ErrorCode::JobIncomplete,
                    "the export has not finished encoding",
                ))
            }
            _ => return Err(changed()),
        }
        let Some(Job {
            generation: job_generation,
            phase,
            ..
        }) = self.job.take()
        else {
            return Err(changed());
        };
        match phase {
            Phase::ExportReady(mut task) => task.finish(job_generation).map_err(BindingError::from),
            _ => Err(changed()),
        }
    }
}

/// Owns the source, sampler cache, renderer and at most one job until disposed.
#[derive(Debug)]
pub(crate) struct Session {
    live: Option<Live>,
}

impl Session {
    /// Takes ownership of `source`; its pixels are moved, never cloned.
    pub(crate) fn new(source: DecodedSource) -> Result<Self, BindingError> {
        let metadata = source.metadata();
        let source = Arc::new(source);
        Ok(Self {
            live: Some(Live {
                metadata,
                sampler: Sampler::new(Arc::clone(&source)),
                source,
                renderer: Renderer::new()?,
                job: None,
                newest: None,
            }),
        })
    }

    fn live_ref(&self) -> Result<&Live, BindingError> {
        self.live.as_ref().ok_or_else(disposed)
    }

    fn live_mut(&mut self) -> Result<&mut Live, BindingError> {
        self.live.as_mut().ok_or_else(disposed)
    }

    pub(crate) fn dispose(&mut self) {
        self.live = None;
    }

    pub(crate) fn start_preview(
        &mut self,
        generation: f64,
        cell_size: f64,
        font_size: f64,
        threshold: f64,
        sampling: Option<&str>,
    ) -> Result<Stage, BindingError> {
        self.start_job(
            generation,
            cell_size,
            font_size,
            threshold,
            sampling,
            Output::Preview,
        )
    }

    pub(crate) fn start_export(
        &mut self,
        generation: f64,
        cell_size: f64,
        font_size: f64,
        threshold: f64,
        sampling: Option<&str>,
    ) -> Result<Stage, BindingError> {
        self.start_job(
            generation,
            cell_size,
            font_size,
            threshold,
            sampling,
            Output::Export,
        )
    }

    fn start_job(
        &mut self,
        generation: f64,
        cell_size: f64,
        font_size: f64,
        threshold: f64,
        sampling: Option<&str>,
        output: Output,
    ) -> Result<Stage, BindingError> {
        let live = self.live_mut()?;
        let generation = safe_integer(generation, "generation")?;
        let settings = PreviewSettings::parse(cell_size, font_size, threshold, sampling)?;
        live.start(generation, settings, output)
    }

    pub(crate) fn drain_export(&mut self, generation: f64) -> Result<Vec<Vec<u8>>, BindingError> {
        let live = self.live_mut()?;
        live.drain_export(safe_integer(generation, "generation")?)
    }

    pub(crate) fn finish_export(&mut self, generation: f64) -> Result<Vec<Vec<u8>>, BindingError> {
        let live = self.live_mut()?;
        live.finish_export(safe_integer(generation, "generation")?)
    }

    pub(crate) fn start_original_preview(
        &mut self,
        generation: f64,
    ) -> Result<Stage, BindingError> {
        let live = self.live_mut()?;
        let generation = safe_integer(generation, "generation")?;
        live.start_original(generation)
    }

    pub(crate) fn advance(
        &mut self,
        generation: f64,
        pixel_budget: f64,
    ) -> Result<Stage, BindingError> {
        let live = self.live_mut()?;
        let generation = safe_integer(generation, "generation")?;
        let budget = positive(pixel_budget, "pixelBudget")?;
        live.advance(generation, budget)
    }

    pub(crate) fn cancel(&mut self, generation: f64) -> Result<(), BindingError> {
        let live = self.live_mut()?;
        live.cancel(safe_integer(generation, "generation")?)
    }

    pub(crate) fn finish(&mut self, generation: f64) -> Result<Frame, BindingError> {
        let live = self.live_mut()?;
        live.finish(safe_integer(generation, "generation")?)
    }

    pub(crate) fn stage(&self) -> Result<Stage, BindingError> {
        let live = self.live_ref()?;
        Ok(live
            .job
            .as_ref()
            .map_or(Stage::Idle, |job| job.phase.stage()))
    }

    pub(crate) fn progress(&self) -> Result<(u64, u64), BindingError> {
        let live = self.live_ref()?;
        Ok(live.job.as_ref().map_or((0, 0), |job| job.phase.progress()))
    }

    /// Returns the immutable post-orientation metadata, or `disposed`.
    pub(crate) fn source_metadata(&self) -> Result<SourceMetadata, BindingError> {
        Ok(self.live_ref()?.metadata)
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Debug;

    use image_to_binary_adapters::ArtworkPngTask;

    use super::*;

    const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

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

    fn code<T: Debug>(result: Result<T, BindingError>) -> ErrorCode {
        result.unwrap_err().code()
    }

    fn start(
        session: &mut Session,
        generation: f64,
        threshold: f64,
    ) -> Result<Stage, BindingError> {
        session.start_preview(generation, 5.0, 10.0, threshold, Some("classic"))
    }

    fn drive(session: &mut Session, generation: f64, budget: f64) -> Frame {
        while session.stage().unwrap() != Stage::Ready {
            session.advance(generation, budget).unwrap();
        }
        session.finish(generation).unwrap()
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
            code(SourcePlan::new(f64::NAN, 2.0, None)),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            code(SourcePlan::new(0.0, 2.0, None)),
            ErrorCode::InvalidDimensions
        );
        assert_eq!(
            code(SourcePlan::new(4_294_967_296.0, 1.0, None)),
            ErrorCode::ResourceLimitExceeded
        );
        assert_eq!(
            code(SourcePlan::new(10_000.0, 5_001.0, None)),
            ErrorCode::ResourceLimitExceeded
        );
        assert_eq!(
            code(SourcePlan::new(2.0, 2.0, Some(3.0))),
            ErrorCode::ResourceLimitExceeded
        );
        assert_eq!(
            code(SourcePlan::new(2.0, 2.0, Some(0.0))),
            ErrorCode::InvalidSettings
        );
        assert_eq!(
            code(SourcePlan::new(2.0, 2.0, Some(50_000_001.0))),
            ErrorCode::InvalidSettings
        );
        let plan = SourcePlan::new(2.0, 2.0, Some(4.0)).unwrap();
        assert_eq!(code(plan.check_len(15)), ErrorCode::InvalidDimensions);
        plan.check_len(16).unwrap();
        assert_eq!(plan.allocate().unwrap().len(), 16);
    }

    #[test]
    fn chunked_sessions_match_the_direct_preview() {
        for (sampling, name, cell, font) in [
            (Sampling::Classic, "classic", 7_u8, 14_u8),
            (Sampling::Average, "average", 6, 18),
        ] {
            let expected = reference(41, 29, cell, font, 128, sampling);
            for budget in [1.0, 17.0, 4096.0, MAX_SAFE] {
                let mut session = new_session(41, 29);
                session
                    .start_preview(0.0, f64::from(cell), f64::from(font), 128.0, Some(name))
                    .unwrap();
                let mut frame = drive(&mut session, 0.0, budget);
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
        assert_eq!(start(&mut session, 0.0, 128.0).unwrap(), Stage::Sampling);
        assert_eq!(session.advance(0.0, 1.0e6).unwrap(), Stage::Preview);
        assert_eq!(session.progress().unwrap().0, 0);
        assert_eq!(session.advance(0.0, 1.0e9).unwrap(), Stage::Ready);
        let (done, total) = session.progress().unwrap();
        assert_eq!(done, total);
        assert_eq!(session.advance(0.0, 1.0).unwrap(), Stage::Ready);
    }

    #[test]
    fn threshold_changes_reuse_the_grid_and_glyph_cache() {
        let mut session = new_session(41, 29);
        assert_eq!(start(&mut session, 0.0, 64.0).unwrap(), Stage::Sampling);
        let mut low = drive(&mut session, 0.0, MAX_SAFE);
        assert_eq!(start(&mut session, 1.0, 192.0).unwrap(), Stage::Preview);
        let mut high = drive(&mut session, 1.0, MAX_SAFE);
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
        let changed = session.start_preview(2.0, 6.0, 10.0, 64.0, Some("classic"));
        assert_eq!(changed.unwrap(), Stage::Sampling);
        assert_eq!(start(&mut session, 3.0, 64.0).unwrap(), Stage::Preview);
    }

    #[test]
    fn cancel_is_terminal_and_a_newer_job_recovers() {
        let expected = reference(30, 30, 5, 10, 128, Sampling::Classic);
        for steps in 0..4 {
            let mut session = new_session(30, 30);
            start(&mut session, 0.0, 128.0).unwrap();
            if steps == 1 {
                session.advance(0.0, 2.0).unwrap();
            } else if steps == 2 {
                session.advance(0.0, MAX_SAFE).unwrap();
                session.advance(0.0, 3.0).unwrap();
            } else if steps == 3 {
                while session.stage().unwrap() != Stage::Ready {
                    session.advance(0.0, MAX_SAFE).unwrap();
                }
            }
            session.cancel(0.0).unwrap();
            assert_eq!(session.stage().unwrap(), Stage::Failed);
            assert_eq!(code(session.advance(0.0, 1.0)), ErrorCode::Cancelled);
            assert_eq!(code(session.finish(0.0)), ErrorCode::Cancelled);
            session.cancel(0.0).unwrap();
            assert_eq!(
                code(start(&mut session, 0.0, 128.0)),
                ErrorCode::StaleGeneration
            );
            start(&mut session, 1.0, 128.0).unwrap();
            let mut frame = drive(&mut session, 1.0, MAX_SAFE);
            assert_eq!(frame.take_pixels().unwrap(), expected.pixels());
        }
    }

    #[test]
    fn cancelled_sampling_leaves_no_partial_cache() {
        let mut session = new_session(41, 29);
        start(&mut session, 0.0, 128.0).unwrap();
        session.advance(0.0, 1.0).unwrap();
        session.cancel(0.0).unwrap();
        assert_eq!(start(&mut session, 1.0, 128.0).unwrap(), Stage::Sampling);
    }

    #[test]
    fn stale_calls_never_disturb_the_current_job() {
        let mut session = new_session(30, 30);
        assert_eq!(code(session.advance(0.0, 1.0)), ErrorCode::NoJob);
        assert_eq!(code(session.cancel(0.0)), ErrorCode::NoJob);
        start(&mut session, 5.0, 128.0).unwrap();
        session.advance(5.0, 3.0).unwrap();
        for generation in [4.0, 5.0] {
            assert_eq!(
                code(start(&mut session, generation, 128.0)),
                ErrorCode::StaleGeneration
            );
        }
        for generation in [4.0, 6.0] {
            assert_eq!(
                code(session.advance(generation, 1.0)),
                ErrorCode::StaleGeneration
            );
            assert_eq!(code(session.finish(generation)), ErrorCode::StaleGeneration);
            assert_eq!(code(session.cancel(generation)), ErrorCode::StaleGeneration);
        }
        assert_eq!(session.progress().unwrap().0, 3);
        start(&mut session, 7.0, 128.0).unwrap();
        assert_eq!(code(session.advance(5.0, 1.0)), ErrorCode::StaleGeneration);
        drive(&mut session, 7.0, MAX_SAFE);
    }

    #[test]
    fn invalid_arguments_frames_and_disposal_are_predictable() {
        let mut session = new_session(30, 30);
        for bad in [f64::NAN, f64::INFINITY, -251.0, 1.5] {
            let cell = session.start_preview(0.0, bad, 10.0, 128.0, Some("classic"));
            assert_eq!(code(cell), ErrorCode::InvalidArgument);
            let threshold = session.start_preview(0.0, 5.0, 10.0, bad, Some("classic"));
            assert_eq!(code(threshold), ErrorCode::InvalidArgument);
        }
        let wide = session.start_preview(0.0, 300.0, 10.0, 128.0, Some("classic"));
        assert_eq!(code(wide), ErrorCode::InvalidSettings);
        let unknown = session.start_preview(0.0, 5.0, 10.0, 128.0, Some("median"));
        assert_eq!(code(unknown), ErrorCode::InvalidSettings);
        let missing = session.start_preview(0.0, 5.0, 10.0, 128.0, None);
        assert_eq!(code(missing), ErrorCode::InvalidArgument);
        assert_eq!(session.stage().unwrap(), Stage::Idle);
        start(&mut session, 0.0, 128.0).unwrap();
        for bad in [0.0, -1.0, f64::NAN, 1.5] {
            assert_eq!(code(session.advance(0.0, bad)), ErrorCode::InvalidArgument);
        }
        assert_eq!(code(session.finish(0.0)), ErrorCode::JobIncomplete);
        let mut frame = drive(&mut session, 0.0, MAX_SAFE);
        assert_eq!(code(session.finish(0.0)), ErrorCode::NoJob);
        let pixels = frame.take_pixels().unwrap();
        assert_eq!(
            pixels.len(),
            usize::try_from(frame.width() * frame.height() * 4).unwrap()
        );
        assert_eq!(code(frame.take_pixels()), ErrorCode::FrameConsumed);
        session.dispose();
        session.dispose();
        assert_eq!(code(start(&mut session, 1.0, 128.0)), ErrorCode::Disposed);
        assert_eq!(code(session.advance(0.0, 1.0)), ErrorCode::Disposed);
        assert_eq!(code(session.finish(0.0)), ErrorCode::Disposed);
        assert_eq!(code(session.cancel(0.0)), ErrorCode::Disposed);
        assert_eq!(code(session.stage()), ErrorCode::Disposed);
        assert_eq!(code(session.progress()), ErrorCode::Disposed);
        assert!(session.live.is_none());
    }

    #[test]
    fn failed_starts_keep_the_current_job_and_the_proposed_generation() {
        let mut session = new_session(41, 29);
        start(&mut session, 5.0, 128.0).unwrap();
        session.advance(5.0, 3.0).unwrap();
        let invalid = PreviewSettings {
            cell_size: 4,
            font_size: 10,
            threshold: 128,
            sampling: Sampling::Classic,
        };
        let failed = session
            .live_mut()
            .unwrap()
            .start(6, invalid, Output::Preview);
        assert_eq!(code(failed), ErrorCode::InvalidSettings);
        assert_eq!(session.live.as_ref().unwrap().newest, Some(5));
        assert_eq!(session.stage().unwrap(), Stage::Sampling);
        assert_eq!(session.progress().unwrap().0, 3);
        assert_eq!(session.advance(5.0, 2.0).unwrap(), Stage::Sampling);
        assert_eq!(session.progress().unwrap().0, 5);
        assert_eq!(start(&mut session, 6.0, 128.0).unwrap(), Stage::Sampling);
        assert_eq!(code(session.advance(5.0, 1.0)), ErrorCode::StaleGeneration);
        let mut frame = drive(&mut session, 6.0, MAX_SAFE);
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
        for budget in [1.0, 17.0, 4096.0, MAX_SAFE] {
            let mut session = new_session(41, 29);
            assert_eq!(session.start_original_preview(0.0).unwrap(), Stage::Preview);
            assert_eq!(session.progress().unwrap(), (0, 41 * 29));
            let mut frame = drive(&mut session, 0.0, budget);
            assert_eq!((frame.width(), frame.height()), (41, 29));
            assert_eq!(frame.take_pixels().unwrap(), gradient(41, 29));
            assert_eq!(code(session.finish(0.0)), ErrorCode::NoJob);
        }
        let mut wide = new_session(1700, 3);
        wide.start_original_preview(0.0).unwrap();
        let frame = drive(&mut wide, 0.0, MAX_SAFE);
        assert_eq!((frame.width(), frame.height()), (1600, 2));
    }

    #[test]
    fn generations_are_monotonic_across_artwork_and_original_jobs() {
        let mut session = new_session(30, 30);
        session.start_original_preview(5.0).unwrap();
        session.advance(5.0, 3.0).unwrap();
        for generation in [4.0, 5.0] {
            assert_eq!(
                code(session.start_original_preview(generation)),
                ErrorCode::StaleGeneration
            );
            assert_eq!(
                code(start(&mut session, generation, 128.0)),
                ErrorCode::StaleGeneration
            );
        }
        assert_eq!(session.stage().unwrap(), Stage::Preview);
        assert_eq!(session.progress().unwrap().0, 3);
        assert_eq!(start(&mut session, 6.0, 128.0).unwrap(), Stage::Sampling);
        assert_eq!(code(session.advance(5.0, 1.0)), ErrorCode::StaleGeneration);
        assert_eq!(session.start_original_preview(7.0).unwrap(), Stage::Preview);
        assert_eq!(
            code(start(&mut session, 7.0, 128.0)),
            ErrorCode::StaleGeneration
        );
        assert_eq!(code(session.advance(6.0, 1.0)), ErrorCode::StaleGeneration);
        let mut frame = drive(&mut session, 7.0, MAX_SAFE);
        assert_eq!(frame.take_pixels().unwrap(), gradient(30, 30));
    }

    #[test]
    fn invalid_starts_keep_an_accepted_original_job() {
        let mut session = new_session(41, 29);
        session.start_original_preview(5.0).unwrap();
        session.advance(5.0, 3.0).unwrap();
        for bad in [f64::NAN, f64::INFINITY, -1.0, 1.5] {
            assert_eq!(
                code(session.start_original_preview(bad)),
                ErrorCode::InvalidArgument
            );
        }
        let invalid = PreviewSettings {
            cell_size: 4,
            font_size: 10,
            threshold: 128,
            sampling: Sampling::Classic,
        };
        let failed = session
            .live_mut()
            .unwrap()
            .start(6, invalid, Output::Preview);
        assert_eq!(code(failed), ErrorCode::InvalidSettings);
        assert_eq!(session.live.as_ref().unwrap().newest, Some(5));
        assert_eq!(session.stage().unwrap(), Stage::Preview);
        assert_eq!(session.progress().unwrap().0, 3);
        assert_eq!(session.advance(5.0, 2.0).unwrap(), Stage::Preview);
        assert_eq!(session.progress().unwrap().0, 5);
        let mut frame = drive(&mut session, 5.0, MAX_SAFE);
        assert_eq!(frame.take_pixels().unwrap(), gradient(41, 29));
    }

    #[test]
    fn original_jobs_share_the_source_and_release_it_on_cancel_and_dispose() {
        let mut session = new_session(41, 29);
        session.start_original_preview(0.0).unwrap();
        assert_eq!(Arc::strong_count(&session.live.as_ref().unwrap().source), 3);
        assert_eq!(code(session.finish(0.0)), ErrorCode::JobIncomplete);
        session.advance(0.0, 2.0).unwrap();
        session.cancel(0.0).unwrap();
        assert_eq!(Arc::strong_count(&session.live.as_ref().unwrap().source), 2);
        assert_eq!(session.stage().unwrap(), Stage::Failed);
        assert_eq!(code(session.advance(0.0, 1.0)), ErrorCode::Cancelled);
        assert_eq!(code(session.finish(0.0)), ErrorCode::Cancelled);
        assert_eq!(
            code(session.start_original_preview(0.0)),
            ErrorCode::StaleGeneration
        );
        assert_eq!(session.start_original_preview(1.0).unwrap(), Stage::Preview);
        session.dispose();
        assert_eq!(
            code(session.start_original_preview(2.0)),
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
        generation: f64,
        sampling: &str,
    ) -> Result<Stage, BindingError> {
        session.start_export(generation, 5.0, 10.0, 128.0, Some(sampling))
    }

    /// Runs an export to completion, collecting drained chunks along the way.
    fn export_bytes(session: &mut Session, generation: f64, budget: f64) -> Vec<u8> {
        let mut bytes = Vec::new();
        while session.stage().unwrap() != Stage::Ready {
            session.advance(generation, budget).unwrap();
            for chunk in session.drain_export(generation).unwrap() {
                bytes.extend(chunk);
            }
        }
        for chunk in session.finish_export(generation).unwrap() {
            bytes.extend(chunk);
        }
        bytes
    }

    fn encoding_row_pixels(session: &Session) -> u64 {
        match &session.live.as_ref().unwrap().job.as_ref().unwrap().phase {
            Phase::Encoding(task) => task.pixels_per_grid_row(),
            other => panic!("expected an encoding export, found {other:?}"),
        }
    }

    fn as_budget(pixels: u64) -> f64 {
        f64::from(u32::try_from(pixels).unwrap())
    }

    /// Samples an export until it reaches `Encoding` and returns one row's pixels.
    fn row_pixels_after_sampling(session: &mut Session, cell: f64, font: f64, name: &str) -> u64 {
        session
            .start_export(0.0, cell, font, 128.0, Some(name))
            .unwrap();
        while session.stage().unwrap() == Stage::Sampling {
            session.advance(0.0, MAX_SAFE).unwrap();
        }
        encoding_row_pixels(session)
    }

    #[test]
    fn exports_match_the_native_png_for_every_budget() {
        for (name, sampling, cell, font) in [
            ("classic", Sampling::Classic, 7_u8, 14_u8),
            ("average", Sampling::Average, 6, 18),
        ] {
            let expected = native_png(41, 29, cell, font, 128, sampling);
            assert_eq!(&expected[..8], &PNG_SIGNATURE);
            assert_eq!(&expected[expected.len() - 12..], &IEND_CHUNK);
            let row = row_pixels_after_sampling(
                &mut new_session(41, 29),
                f64::from(cell),
                f64::from(font),
                name,
            );
            let budgets = [
                1.0,
                as_budget(row.saturating_sub(1).max(1)),
                as_budget(row),
                as_budget(3 * row + 1),
                17.0,
                4096.0,
                MAX_SAFE,
            ];
            for budget in budgets {
                let mut session = new_session(41, 29);
                session
                    .start_export(0.0, f64::from(cell), f64::from(font), 128.0, Some(name))
                    .unwrap();
                assert_eq!(export_bytes(&mut session, 0.0, budget), expected);
            }
        }
    }

    #[test]
    fn export_budgets_convert_pixels_into_whole_grid_rows() {
        let mut session = new_session(30, 30);
        let row = row_pixels_after_sampling(&mut session, 5.0, 10.0, "classic");
        assert_eq!(session.progress().unwrap(), (0, 6));
        assert_eq!(session.advance(0.0, 1.0).unwrap(), Stage::Encoding);
        assert_eq!(session.progress().unwrap().0, 1);
        assert_eq!(
            session.advance(0.0, as_budget(4 * row - 1)).unwrap(),
            Stage::Encoding
        );
        assert_eq!(session.progress().unwrap().0, 4);
        assert_eq!(
            session.advance(0.0, as_budget(2 * row)).unwrap(),
            Stage::Ready
        );
        assert_eq!(session.progress().unwrap(), (6, 6));
    }

    #[test]
    fn mid_sized_budgets_leave_a_cancellation_boundary_before_publishing() {
        let expected = native_png(300, 200, 5, 10, 128, Sampling::Classic);
        let mut session = new_session(300, 200);
        let row = row_pixels_after_sampling(&mut session, 5.0, 10.0, "classic");
        let budget = as_budget(4 * row);
        assert_eq!(session.advance(0.0, budget).unwrap(), Stage::Encoding);
        assert_eq!(session.progress().unwrap().0, 4);
        session.cancel(0.0).unwrap();
        assert_eq!(code(session.advance(0.0, budget)), ErrorCode::Cancelled);
        assert_eq!(code(session.drain_export(0.0)), ErrorCode::Cancelled);
        assert_eq!(code(session.finish_export(0.0)), ErrorCode::Cancelled);

        let mut fresh = new_session(300, 200);
        start_export(&mut fresh, 1.0, "classic").unwrap();
        let mut advances = 0_u32;
        while fresh.stage().unwrap() != Stage::Ready {
            fresh.advance(1.0, budget).unwrap();
            advances += 1;
        }
        assert!(
            advances > 1,
            "a mid-sized budget must need several advances"
        );
        let mut restarted = new_session(300, 200);
        start_export(&mut restarted, 1.0, "classic").unwrap();
        assert_eq!(export_bytes(&mut restarted, 1.0, budget), expected);
    }

    #[test]
    fn export_stages_run_sampling_then_encoding_then_ready() {
        let mut session = new_session(30, 30);
        assert_eq!(
            start_export(&mut session, 0.0, "classic").unwrap(),
            Stage::Sampling
        );
        while session.stage().unwrap() == Stage::Sampling {
            session.advance(0.0, MAX_SAFE).unwrap();
        }
        assert_eq!(session.stage().unwrap(), Stage::Encoding);
        assert_eq!(session.progress().unwrap().0, 0);
        assert_eq!(session.advance(0.0, 2.0).unwrap(), Stage::Encoding);
        assert_eq!(session.progress().unwrap().0, 1);
        assert_eq!(session.advance(0.0, MAX_SAFE).unwrap(), Stage::Ready);
        let (done, total) = session.progress().unwrap();
        assert_eq!(done, total);
        assert_eq!(session.advance(0.0, 1.0).unwrap(), Stage::Ready);
        assert!(!session.finish_export(0.0).unwrap().is_empty());
        assert_eq!(session.stage().unwrap(), Stage::Idle);
    }

    #[test]
    fn export_drains_are_in_order_and_exactly_once() {
        let expected = native_png(41, 29, 5, 10, 128, Sampling::Classic);
        let mut session = new_session(41, 29);
        start_export(&mut session, 0.0, "classic").unwrap();
        let mut drained = Vec::new();
        while session.stage().unwrap() != Stage::Ready {
            session.advance(0.0, 3.0).unwrap();
            drained.extend(session.drain_export(0.0).unwrap());
            assert!(session.drain_export(0.0).unwrap().is_empty());
        }
        let tail = session.finish_export(0.0).unwrap();
        assert_eq!(code(session.finish_export(0.0)), ErrorCode::NoJob);
        let joined: Vec<u8> = drained.into_iter().chain(tail).flatten().collect();
        assert_eq!(joined, expected);
    }

    #[test]
    fn export_sampling_reuses_the_preview_grid_cache() {
        let expected = native_png(41, 29, 5, 10, 128, Sampling::Classic);
        let mut session = new_session(41, 29);
        start(&mut session, 0.0, 128.0).unwrap();
        drive(&mut session, 0.0, MAX_SAFE);
        assert_eq!(
            start_export(&mut session, 1.0, "classic").unwrap(),
            Stage::Encoding
        );
        assert_eq!(export_bytes(&mut session, 1.0, 9.0), expected);
    }

    #[test]
    fn export_usage_errors_keep_the_job_and_use_stable_codes() {
        let mut session = new_session(30, 30);
        assert_eq!(code(session.drain_export(0.0)), ErrorCode::NoJob);
        assert_eq!(code(session.finish_export(0.0)), ErrorCode::NoJob);
        start_export(&mut session, 0.0, "classic").unwrap();
        assert_eq!(code(session.advance(0.0, 0.0)), ErrorCode::InvalidArgument);
        assert_eq!(session.stage().unwrap(), Stage::Sampling);
        assert_eq!(code(session.finish_export(0.0)), ErrorCode::JobIncomplete);
        assert!(session.drain_export(0.0).unwrap().is_empty());
        assert_eq!(code(session.finish(0.0)), ErrorCode::NoJob);
        assert_eq!(
            code(start_export(&mut session, 0.0, "classic")),
            ErrorCode::StaleGeneration
        );
        assert_eq!(code(session.drain_export(1.0)), ErrorCode::StaleGeneration);
        assert_eq!(code(session.finish_export(1.0)), ErrorCode::StaleGeneration);
        let invalid = session.start_export(1.0, 4.0, 10.0, 128.0, Some("classic"));
        assert_eq!(code(invalid), ErrorCode::InvalidSettings);
        assert_eq!(session.stage().unwrap(), Stage::Sampling);
        while session.stage().unwrap() != Stage::Encoding {
            session.advance(0.0, MAX_SAFE).unwrap();
        }
        assert_eq!(code(session.finish_export(0.0)), ErrorCode::JobIncomplete);
        session.advance(0.0, 1.0).unwrap();
        assert_eq!(code(session.finish_export(0.0)), ErrorCode::JobIncomplete);
        assert_eq!(session.stage().unwrap(), Stage::Encoding);
        export_bytes(&mut session, 0.0, MAX_SAFE);
        assert_eq!(code(session.finish_export(0.0)), ErrorCode::NoJob);
    }

    #[test]
    fn preview_and_export_calls_never_act_on_the_other_kind() {
        let mut session = new_session(30, 30);
        start_export(&mut session, 0.0, "classic").unwrap();
        session.advance(0.0, MAX_SAFE).unwrap();
        assert_eq!(code(session.finish(0.0)), ErrorCode::NoJob);
        assert_eq!(session.stage().unwrap(), Stage::Encoding);
        session.advance(0.0, MAX_SAFE).unwrap();
        assert_eq!(code(session.finish(0.0)), ErrorCode::NoJob);
        assert_eq!(session.stage().unwrap(), Stage::Ready);
        assert!(!session.finish_export(0.0).unwrap().is_empty());

        start(&mut session, 1.0, 128.0).unwrap();
        assert_eq!(code(session.finish_export(1.0)), ErrorCode::NoJob);
        assert_eq!(code(session.drain_export(1.0)), ErrorCode::NoJob);
        assert_eq!(session.stage().unwrap(), Stage::Preview);
        drive(&mut session, 1.0, MAX_SAFE);
        assert_eq!(code(session.drain_export(1.0)), ErrorCode::NoJob);
        assert_eq!(code(session.finish_export(1.0)), ErrorCode::NoJob);
    }

    #[test]
    fn exports_and_previews_supersede_each_other_by_generation() {
        let expected = native_png(30, 30, 5, 10, 128, Sampling::Classic);
        let mut session = new_session(30, 30);
        start_export(&mut session, 0.0, "classic").unwrap();
        session.advance(0.0, 3.0).unwrap();
        assert_eq!(start(&mut session, 1.0, 128.0).unwrap(), Stage::Sampling);
        assert_eq!(code(session.advance(0.0, 1.0)), ErrorCode::StaleGeneration);
        assert_eq!(code(session.drain_export(0.0)), ErrorCode::StaleGeneration);
        assert_eq!(code(session.finish_export(0.0)), ErrorCode::StaleGeneration);
        assert_eq!(code(session.cancel(0.0)), ErrorCode::StaleGeneration);
        drive(&mut session, 1.0, MAX_SAFE);

        start_export(&mut session, 2.0, "classic").unwrap();
        assert_eq!(code(session.advance(1.0, 1.0)), ErrorCode::StaleGeneration);
        assert_eq!(code(session.finish(1.0)), ErrorCode::StaleGeneration);
        assert_eq!(export_bytes(&mut session, 2.0, MAX_SAFE), expected);
    }

    #[test]
    fn cancelled_exports_are_terminal_and_a_newer_export_recovers() {
        let expected = native_png(30, 30, 5, 10, 128, Sampling::Classic);
        for steps in 0..3 {
            let mut session = new_session(30, 30);
            start_export(&mut session, 0.0, "classic").unwrap();
            for _ in 0..steps {
                session.advance(0.0, MAX_SAFE).unwrap();
            }
            session.cancel(0.0).unwrap();
            session.cancel(0.0).unwrap();
            assert_eq!(session.stage().unwrap(), Stage::Failed);
            assert_eq!(code(session.advance(0.0, 1.0)), ErrorCode::Cancelled);
            assert_eq!(code(session.drain_export(0.0)), ErrorCode::Cancelled);
            assert_eq!(code(session.finish_export(0.0)), ErrorCode::Cancelled);
            assert_eq!(
                code(start_export(&mut session, 0.0, "classic")),
                ErrorCode::StaleGeneration
            );
            start_export(&mut session, 1.0, "classic").unwrap();
            assert_eq!(export_bytes(&mut session, 1.0, 5.0), expected);
        }
    }

    #[test]
    fn disposal_mid_export_releases_the_job_and_every_call_is_disposed() {
        let mut session = new_session(30, 30);
        start_export(&mut session, 0.0, "average").unwrap();
        session.advance(0.0, MAX_SAFE).unwrap();
        session.advance(0.0, 2.0).unwrap();
        session.dispose();
        session.dispose();
        assert!(session.live.is_none());
        assert_eq!(code(session.drain_export(0.0)), ErrorCode::Disposed);
        assert_eq!(code(session.finish_export(0.0)), ErrorCode::Disposed);
        assert_eq!(code(session.advance(0.0, 1.0)), ErrorCode::Disposed);
        assert_eq!(
            code(start_export(&mut session, 1.0, "classic")),
            ErrorCode::Disposed
        );
        assert_eq!(code(session.stage()), ErrorCode::Disposed);
    }

    #[test]
    fn wrong_kind_calls_report_no_job_even_when_the_job_failed() {
        let mut preview = new_session(30, 30);
        start(&mut preview, 0.0, 128.0).unwrap();
        preview.cancel(0.0).unwrap();
        assert_eq!(code(preview.drain_export(0.0)), ErrorCode::NoJob);
        assert_eq!(code(preview.finish_export(0.0)), ErrorCode::NoJob);
        assert_eq!(code(preview.finish(0.0)), ErrorCode::Cancelled);
        assert_eq!(preview.stage().unwrap(), Stage::Failed);

        let mut export = new_session(30, 30);
        start_export(&mut export, 0.0, "classic").unwrap();
        export.cancel(0.0).unwrap();
        assert_eq!(code(export.finish(0.0)), ErrorCode::NoJob);
        assert_eq!(code(export.finish_export(0.0)), ErrorCode::Cancelled);
        assert_eq!(export.stage().unwrap(), Stage::Failed);
        assert_eq!(code(export.drain_export(0.0)), ErrorCode::Cancelled);
        assert_eq!(
            code(start_export(&mut export, 0.0, "classic")),
            ErrorCode::StaleGeneration
        );
        assert_eq!(
            start_export(&mut export, 1.0, "classic").unwrap(),
            Stage::Sampling
        );
    }
}
