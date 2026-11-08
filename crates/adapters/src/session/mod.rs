//! Transport-neutral processing session: one still source or one animation of frames, one
//! renderer, and at most one job.
//!
//! This module is the single job state machine behind the WASM preview and export bindings
//! and the native desktop shell. Both shells call the same methods with the same limits, so
//! previews and PNG exports are deterministic across them.
//!
//! Invariants:
//! - At least one frame. A still source has exactly one frame with a classic sample cache;
//!   every animation frame has its own sampler with the cache disabled.
//! - One renderer per [`Session`]; at most one job, which names the frame it renders.
//! - A job generation is accepted only when strictly newer than every earlier accepted
//!   generation. Calls naming any other generation are stale and leave the job untouched.
//! - A failed start keeps the current job; a failed advance makes that job terminal until a
//!   newer start replaces it.
//! - Disposal releases the frames, caches, renderer and job; every later call reports
//!   [`SessionError::Disposed`].
//!
//! Failure contract: every fallible method returns a [`SessionError`] whose
//! [`ErrorCode`] is the stable vocabulary shared by both shells. No method panics.

#[cfg(test)]
mod animation_tests;
pub mod args;
#[cfg(test)]
mod contract_tests;
mod error;
mod frame;
mod job;
mod limits;
#[cfg(test)]
mod parity_tests;
mod plan;
mod settings;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use image_to_binary_core::{ContractError, Generation, Sampling, SourceMetadata};
use image_to_binary_processing::{
    DecodedAnimation, DecodedSource, OriginalPreviewTask, RenderOptions, Renderer, Sampler,
    SamplingPlan,
};

pub use error::{EngineFailure, ErrorCode, JobKind, SessionError, StaleReason};
pub use frame::Frame;
pub use image_to_binary_processing::LoopCount;
pub use job::{OutputKind, Stage};
pub use limits::{
    DEFAULT_PIXEL_BUDGET, EXPORT_CHUNK_BYTES, EXPORT_LIMITS, EXPORT_MAX_ENCODED_BYTES,
    EXPORT_MAX_PENDING_BYTES, MAX_DECODED_BYTES, MAX_ENCODED_BYTES, MAX_SOURCE_PIXELS,
};
pub use plan::{EncodedPlan, SourcePlan};
pub use settings::PreviewSettings;

use job::{Job, Phase};

/// One frame's pixels, the sampler that reads them and the cache policy it runs under.
#[derive(Debug)]
struct FrameSlot {
    source: Arc<DecodedSource>,
    sampler: Sampler,
}

impl FrameSlot {
    /// The only frame of a still source keeps the default sample cache.
    fn still(source: Arc<DecodedSource>) -> Self {
        Self {
            sampler: Sampler::new(Arc::clone(&source)),
            source,
        }
    }

    /// Animation frames have no sample cache, so a long animation retains no sampled grids;
    /// every preview or export samples its frame again, and the pixels stay in `source` only.
    fn animated(source: Arc<DecodedSource>) -> Self {
        Self {
            sampler: Sampler::with_cache_capacity(Arc::clone(&source), 0),
            source,
        }
    }
}

#[derive(Debug)]
struct Live {
    metadata: SourceMetadata,
    frames: Vec<FrameSlot>,
    /// Empty for a still source; otherwise one delay per frame.
    delays_centiseconds: Vec<u16>,
    loop_count: LoopCount,
    renderer: Renderer,
    job: Option<Job>,
    newest: Option<u64>,
}

impl Live {
    fn slot(&self, frame: usize) -> Result<&FrameSlot, SessionError> {
        self.frames.get(frame).ok_or(SessionError::FrameOutOfRange {
            frame,
            count: self.frames.len(),
        })
    }

    fn slot_mut(&mut self, frame: usize) -> Result<&mut FrameSlot, SessionError> {
        let count = self.frames.len();
        self.frames
            .get_mut(frame)
            .ok_or(SessionError::FrameOutOfRange { frame, count })
    }

    fn check_generation(&self, generation: Generation) -> Result<(), SessionError> {
        match self.newest {
            Some(newest) if newest == generation.value() => Ok(()),
            Some(_) => Err(SessionError::StaleGeneration(StaleReason::NotCurrent)),
            None => Err(SessionError::NoJob),
        }
    }

    fn start(
        &mut self,
        generation: Generation,
        frame: usize,
        settings: PreviewSettings,
        kind: JobKind,
        output: OutputKind,
    ) -> Result<Stage, SessionError> {
        self.slot(frame)?;
        self.ensure_newer(generation)?;
        let job = self.begin(generation, frame, settings, kind, output)?;
        Ok(self.commit(generation, job))
    }

    fn start_original(
        &mut self,
        generation: Generation,
        frame: usize,
    ) -> Result<Stage, SessionError> {
        let source = Arc::clone(&self.slot(frame)?.source);
        self.ensure_newer(generation)?;
        let task = OriginalPreviewTask::new(source, generation)?;
        let job = Job {
            generation,
            kind: JobKind::Preview,
            frame,
            phase: Phase::Original(Box::new(task)),
        };
        Ok(self.commit(generation, job))
    }

    fn ensure_newer(&self, generation: Generation) -> Result<(), SessionError> {
        if self
            .newest
            .is_some_and(|newest| generation.value() <= newest)
        {
            return Err(SessionError::StaleGeneration(StaleReason::NotNewer));
        }
        Ok(())
    }

    fn commit(&mut self, generation: Generation, job: Job) -> Stage {
        let stage = job.phase.stage();
        self.newest = Some(generation.value());
        self.job = Some(job);
        stage
    }

    fn begin(
        &mut self,
        generation: Generation,
        frame: usize,
        settings: PreviewSettings,
        kind: JobKind,
        output: OutputKind,
    ) -> Result<Job, SessionError> {
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
            mode: settings.mode,
            style: settings.style,
            ..RenderOptions::default()
        };
        let plan = self.slot_mut(frame)?.sampler.prepare(
            settings.cell_size,
            cell_height,
            settings.sampling,
            generation,
        )?;
        let phase = match (plan, kind) {
            (SamplingPlan::Cached(grid), JobKind::Preview) => Phase::Preview(Box::new(
                self.renderer.preview_task(grid, options, generation)?,
            )),
            (SamplingPlan::Cached(grid), JobKind::Export) => {
                job::export_phase(grid, options, generation, output)?
            }
            (SamplingPlan::Task(task), kind) => Phase::Sampling(task, options, kind, output),
        };
        Ok(Job {
            generation,
            kind,
            frame,
            phase,
        })
    }

    fn advance(&mut self, generation: Generation, budget: u64) -> Result<Stage, SessionError> {
        self.check_generation(generation)?;
        let Self {
            frames,
            renderer,
            job,
            ..
        } = self;
        let job = job.as_mut().ok_or(SessionError::NoJob)?;
        let frame = job.frame;
        let count = frames.len();
        let slot = frames
            .get_mut(frame)
            .ok_or(SessionError::FrameOutOfRange { frame, count })?;
        job.advance(&mut slot.sampler, renderer, budget)
    }

    fn cancel(&mut self, generation: Generation) -> Result<(), SessionError> {
        self.check_generation(generation)?;
        if let Some(job) = self.job.as_mut() {
            if !matches!(job.phase, Phase::Failed(_)) {
                job.phase = Phase::Failed(SessionError::Contract(ContractError::Cancelled));
            }
        }
        Ok(())
    }

    fn finish(&mut self, generation: Generation) -> Result<Frame, SessionError> {
        self.check_generation(generation)?;
        let job = self.job.as_ref().ok_or(SessionError::NoJob)?;
        if job.kind != JobKind::Preview {
            return Err(SessionError::NoActiveJob(JobKind::Preview));
        }
        match &job.phase {
            Phase::Ready(_) | Phase::OriginalReady(_) => {}
            Phase::Failed(error) => return Err(error.clone()),
            Phase::Sampling(..) | Phase::Preview(_) | Phase::Original(_) => {
                return Err(SessionError::JobIncomplete(JobKind::Preview));
            }
            Phase::Encoding(_) | Phase::ExportReady(_) => return Err(SessionError::Changed),
        }
        let Some(Job {
            generation: job_generation,
            phase,
            ..
        }) = self.job.take()
        else {
            return Err(SessionError::Changed);
        };
        match phase {
            Phase::Ready(task) => Ok(Frame::new((*task).finish(job_generation)?)),
            Phase::OriginalReady(task) => Ok(Frame::new((*task).finish(job_generation)?)),
            _ => Err(SessionError::Changed),
        }
    }

    fn drain_export(&mut self, generation: Generation) -> Result<Vec<Vec<u8>>, SessionError> {
        self.check_generation(generation)?;
        let job = self.job.as_mut().ok_or(SessionError::NoJob)?;
        if job.kind != JobKind::Export {
            return Err(SessionError::NoActiveJob(JobKind::Export));
        }
        let drained = match &mut job.phase {
            Phase::Sampling(..) => return Ok(Vec::new()),
            Phase::Encoding(task) | Phase::ExportReady(task) => {
                task.drain_chunks().map_err(SessionError::from)
            }
            Phase::Failed(error) => return Err(error.clone()),
            _ => return Err(SessionError::Changed),
        };
        match drained {
            Ok(chunks) => Ok(chunks),
            Err(error) => {
                job.phase = Phase::Failed(error.clone());
                Err(error)
            }
        }
    }

    fn finish_export(&mut self, generation: Generation) -> Result<Vec<Vec<u8>>, SessionError> {
        self.check_generation(generation)?;
        let job = self.job.as_ref().ok_or(SessionError::NoJob)?;
        if job.kind != JobKind::Export {
            return Err(SessionError::NoActiveJob(JobKind::Export));
        }
        match &job.phase {
            Phase::ExportReady(_) => {}
            Phase::Failed(error) => return Err(error.clone()),
            Phase::Sampling(..) | Phase::Encoding(_) => {
                return Err(SessionError::JobIncomplete(JobKind::Export));
            }
            _ => return Err(SessionError::Changed),
        }
        let Some(Job {
            generation: job_generation,
            phase,
            ..
        }) = self.job.take()
        else {
            return Err(SessionError::Changed);
        };
        match phase {
            Phase::ExportReady(mut task) => task.finish(job_generation).map_err(SessionError::from),
            _ => Err(SessionError::Changed),
        }
    }
}

/// Owns the frames, their sampler caches, the renderer and at most one job until disposed.
///
/// Invariants and failure contract are described at the module level. Methods take
/// [`Generation`] and typed [`PreviewSettings`], so no caller can pass an unchecked number.
#[derive(Debug)]
pub struct Session {
    live: Option<Live>,
}

impl Session {
    /// Takes ownership of `source` as a single-frame still; its pixels are moved, never cloned.
    ///
    /// # Errors
    ///
    /// Returns the renderer's failure when its fonts cannot be prepared.
    pub fn new(source: DecodedSource) -> Result<Self, SessionError> {
        let metadata = source.metadata();
        Ok(Self {
            live: Some(Live {
                metadata,
                frames: vec![FrameSlot::still(Arc::new(source))],
                delays_centiseconds: Vec::new(),
                loop_count: LoopCount::Finite(0),
                renderer: Renderer::new()?,
                job: None,
                newest: None,
            }),
        })
    }

    /// Takes ownership of an animation's frames, which share their pixels with `animation`.
    /// Frame order and delays are kept; the loop count is the decoder's.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::FrameTableReservation`] when the frame table cannot be
    /// reserved, or the renderer's failure when its fonts cannot be prepared.
    pub fn new_animation(animation: &DecodedAnimation) -> Result<Self, SessionError> {
        let metadata = animation.metadata();
        let loop_count = animation.loop_count();
        let count = animation.frames().len();
        let mut frames = Vec::new();
        frames
            .try_reserve_exact(count)
            .map_err(|_| SessionError::FrameTableReservation)?;
        let mut delays_centiseconds = Vec::new();
        delays_centiseconds
            .try_reserve_exact(count)
            .map_err(|_| SessionError::FrameTableReservation)?;
        for frame in animation.frames() {
            frames.push(FrameSlot::animated(Arc::clone(frame.source())));
            delays_centiseconds.push(frame.delay_centiseconds());
        }
        Ok(Self {
            live: Some(Live {
                metadata,
                frames,
                delays_centiseconds,
                loop_count,
                renderer: Renderer::new()?,
                job: None,
                newest: None,
            }),
        })
    }

    fn live_ref(&self) -> Result<&Live, SessionError> {
        self.live.as_ref().ok_or(SessionError::Disposed)
    }

    fn live_mut(&mut self) -> Result<&mut Live, SessionError> {
        self.live.as_mut().ok_or(SessionError::Disposed)
    }

    /// Returns `Ok` while the session holds its resources, or [`SessionError::Disposed`].
    /// Callers that validate arguments first call this so disposal is reported before
    /// argument errors, as both shells require.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Disposed`] after [`Session::dispose`].
    pub fn ensure_live(&self) -> Result<(), SessionError> {
        self.live_ref().map(|_| ())
    }

    /// Releases the source, caches, renderer and job. Idempotent.
    pub fn dispose(&mut self) {
        self.live = None;
    }

    /// Starts a preview of frame `0`, superseding any current job. Allocation and font setup
    /// run synchronously; sampling runs in later advances unless the grid is cached.
    ///
    /// # Errors
    ///
    /// Same as [`Session::start_preview_frame`].
    pub fn start_preview(
        &mut self,
        generation: Generation,
        settings: PreviewSettings,
    ) -> Result<Stage, SessionError> {
        self.start_preview_frame(generation, 0, settings)
    }

    /// Starts a preview of `frame` that supersedes any current job. A still source accepts
    /// only frame `0`; an animation accepts `0..frame_count()`.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Disposed`], [`SessionError::FrameOutOfRange`] for an index
    /// outside the frames, [`SessionError::StaleGeneration`] for a generation not newer than
    /// the last accepted one, or a resource or contract error. A failed start keeps the
    /// current job.
    pub fn start_preview_frame(
        &mut self,
        generation: Generation,
        frame: usize,
        settings: PreviewSettings,
    ) -> Result<Stage, SessionError> {
        self.live_mut()?.start(
            generation,
            frame,
            settings,
            JobKind::Preview,
            OutputKind::Png,
        )
    }

    /// Starts a PNG export of frame `0`. It shares the preview sample cache for the same settings.
    ///
    /// # Errors
    ///
    /// Same as [`Session::start_preview`].
    pub fn start_export(
        &mut self,
        generation: Generation,
        settings: PreviewSettings,
    ) -> Result<Stage, SessionError> {
        self.start_export_as(generation, settings, OutputKind::Png)
    }

    /// Starts an export of frame `0` in the given output kind over the same sampled grid and
    /// settings as [`Session::start_preview`], so PNG and text describe one grid.
    ///
    /// # Errors
    ///
    /// Same as [`Session::start_preview`].
    pub fn start_export_as(
        &mut self,
        generation: Generation,
        settings: PreviewSettings,
        output: OutputKind,
    ) -> Result<Stage, SessionError> {
        self.start_export_frame_as(generation, 0, settings, output)
    }

    /// Starts an export of `frame` in the given output kind, with the same frame rules and
    /// errors as [`Session::start_preview_frame`].
    ///
    /// # Errors
    ///
    /// Same as [`Session::start_preview_frame`].
    pub fn start_export_frame_as(
        &mut self,
        generation: Generation,
        frame: usize,
        settings: PreviewSettings,
        output: OutputKind,
    ) -> Result<Stage, SessionError> {
        self.live_mut()?
            .start(generation, frame, settings, JobKind::Export, output)
    }

    /// Starts a bounded original-image preview of frame `0` that supersedes any current job.
    ///
    /// # Errors
    ///
    /// Same as [`Session::start_original_preview_frame`].
    pub fn start_original_preview(
        &mut self,
        generation: Generation,
    ) -> Result<Stage, SessionError> {
        self.start_original_preview_frame(generation, 0)
    }

    /// Starts a bounded original-image preview of `frame` that supersedes any current job.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Disposed`], [`SessionError::FrameOutOfRange`],
    /// [`SessionError::StaleGeneration`], or a resource error; a failed start keeps the
    /// current job.
    pub fn start_original_preview_frame(
        &mut self,
        generation: Generation,
        frame: usize,
    ) -> Result<Stage, SessionError> {
        self.live_mut()?.start_original(generation, frame)
    }

    /// Returns the number of frames: `1` for a still, at least two for an animation.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Disposed`] after disposal.
    pub fn frame_count(&self) -> Result<usize, SessionError> {
        Ok(self.live_ref()?.frames.len())
    }

    /// Returns each frame's display time in centiseconds, in presentation order. Empty for a
    /// still, so the slice length is `frame_count()` only for an animation.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Disposed`] after disposal.
    pub fn frame_delays_centiseconds(&self) -> Result<&[u16], SessionError> {
        Ok(&self.live_ref()?.delays_centiseconds)
    }

    /// Returns how many times the animation repeats. A still reports `Finite(0)`, which plays
    /// once.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Disposed`] after disposal.
    pub fn loop_count(&self) -> Result<LoopCount, SessionError> {
        Ok(self.live_ref()?.loop_count)
    }

    /// Spends `pixel_budget` pixels of work in the current stage only.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::ZeroBudget`] for a zero budget, [`SessionError::NoJob`],
    /// [`SessionError::StaleGeneration`], [`SessionError::Disposed`], or the terminal
    /// failure stored by a previous step. A zero budget leaves the job unchanged.
    pub fn advance(
        &mut self,
        generation: Generation,
        pixel_budget: u64,
    ) -> Result<Stage, SessionError> {
        let live = self.live_mut()?;
        if pixel_budget == 0 {
            return Err(SessionError::ZeroBudget);
        }
        live.advance(generation, pixel_budget)
    }

    /// Cancels the job and releases its partial work. Repeating it is a no-op.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::NoJob`], [`SessionError::StaleGeneration`] or
    /// [`SessionError::Disposed`].
    pub fn cancel(&mut self, generation: Generation) -> Result<(), SessionError> {
        self.live_mut()?.cancel(generation)
    }

    /// Returns the finished preview frame once, when the job is a preview in `Ready`.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::JobIncomplete`] before rendering finishes,
    /// [`SessionError::NoActiveJob`] for an export, [`SessionError::NoJob`],
    /// [`SessionError::StaleGeneration`], [`SessionError::Disposed`], or the job's
    /// terminal failure. An export job is left untouched.
    pub fn finish(&mut self, generation: Generation) -> Result<Frame, SessionError> {
        self.live_mut()?.finish(generation)
    }

    /// Returns the export chunks completed since the last drain, in order, each once.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::NoActiveJob`] for a preview, [`SessionError::NoJob`],
    /// [`SessionError::StaleGeneration`], [`SessionError::Disposed`], or the encoder or
    /// storage failure, which is stored as the job's terminal failure.
    pub fn drain_export(&mut self, generation: Generation) -> Result<Vec<Vec<u8>>, SessionError> {
        self.live_mut()?.drain_export(generation)
    }

    /// Finalizes the PNG once and returns every chunk not yet drained. The concatenation of
    /// all drained and final chunks is the complete PNG file. A successful finish releases
    /// the job, so a second finish reports [`SessionError::NoJob`].
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::JobIncomplete`] before the last row is encoded,
    /// [`SessionError::NoActiveJob`] for a preview, [`SessionError::NoJob`],
    /// [`SessionError::StaleGeneration`], [`SessionError::Disposed`], or the terminal
    /// failure. A failed job is not released by this call.
    pub fn finish_export(&mut self, generation: Generation) -> Result<Vec<Vec<u8>>, SessionError> {
        self.live_mut()?.finish_export(generation)
    }

    /// Returns the current job stage, [`Stage::Idle`] when there is none.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Disposed`] after disposal.
    pub fn stage(&self) -> Result<Stage, SessionError> {
        let live = self.live_ref()?;
        Ok(live
            .job
            .as_ref()
            .map_or(Stage::Idle, |job| job.phase.stage()))
    }

    /// Returns `(completed, total)` work in the current stage, in that stage's unit:
    /// sampled cells or source pixels while sampling, output pixels while rendering a
    /// preview or original, and grid rows while an export encodes. `(0, 0)` when there is no job.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Disposed`] after disposal.
    pub fn progress(&self) -> Result<(u64, u64), SessionError> {
        let live = self.live_ref()?;
        Ok(live.job.as_ref().map_or((0, 0), |job| job.phase.progress()))
    }

    /// Returns the immutable post-orientation source metadata.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Disposed`] after disposal.
    pub fn source_metadata(&self) -> Result<SourceMetadata, SessionError> {
        Ok(self.live_ref()?.metadata)
    }
}
