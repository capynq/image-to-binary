//! The single job slot: sampling, then preview or encoding, then ready or failed.
//!
//! Each `advance` spends its pixel budget in the current stage only. The call that completes a stage
//! performs that stage's non-interruptible setup and no next-stage work. A GIF export stays in
//! `Encoding` for every frame: it samples, renders and encodes one frame at a time, so its progress
//! counts frames rather than grid rows.

use std::mem;
use std::sync::Arc;

use image_to_binary_core::{ContractError, Generation, Progress, Sampling};
use image_to_binary_processing::{
    OriginalPreviewTask, PreviewAdvance, PreviewTask, RenderOptions, Renderer, SampleGrid, Sampler,
    SamplingAdvance, SamplingPlan, SamplingTask,
};

use super::error::{JobKind, SessionError};
use super::limits::EXPORT_LIMITS;
use super::FrameSlot;
use crate::{ArtworkGifTask, ArtworkPngTask, ArtworkTextTask, PngTaskError};

/// Position of the current job in the pipeline. Export jobs pass through `Encoding`
/// before `Ready`; a `Failed` job keeps its error until a newer start replaces it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    /// No job is held.
    Idle,
    /// Sampling the source grid; progress counts cells or source pixels.
    Sampling,
    /// Rendering a preview; progress counts output pixels.
    Preview,
    /// Encoding export rows or, for GIF, frames; progress counts grid rows or frames.
    Encoding,
    /// The job finished. A preview can be taken once; an export can be finished.
    Ready,
    /// Terminal failure. The error is returned until a newer start replaces the job.
    Failed,
}

/// The file encoding an export job produces. `Png` is the default; `Text` produces
/// UTF-8 grid rows over the same sampled grid and render options; `Gif` encodes every
/// frame of an animation, one frame at a time.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OutputKind {
    /// A PNG image.
    #[default]
    Png,
    /// UTF-8 text, one grid row per line, each line ending in LF.
    Text,
    /// An animated GIF with one frame per source frame. Its progress counts frames, and it
    /// takes no frame index.
    Gif,
}

#[derive(Debug)]
pub(super) enum Phase {
    Sampling(Box<SamplingTask>, RenderOptions, JobKind, OutputKind),
    Preview(Box<PreviewTask>),
    Ready(Box<PreviewTask>),
    Original(Box<OriginalPreviewTask>),
    OriginalReady(Box<OriginalPreviewTask>),
    Encoding(Box<ExportTask>),
    ExportReady(Box<ExportTask>),
    Failed(SessionError),
}

impl Phase {
    pub(super) fn stage(&self) -> Stage {
        match self {
            Self::Sampling(..) => Stage::Sampling,
            Self::Preview(_) | Self::Original(_) => Stage::Preview,
            Self::Encoding(_) => Stage::Encoding,
            Self::Ready(_) | Self::OriginalReady(_) | Self::ExportReady(_) => Stage::Ready,
            Self::Failed(_) => Stage::Failed,
        }
    }

    pub(super) fn progress(&self) -> (u64, u64) {
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

/// One export encoder, chosen once when the job starts encoding. Raster exports share the
/// grid, render options, limits and lifecycle, and differ only in row encoding.
#[derive(Debug)]
pub(super) enum ExportTask {
    Raster(RasterTask),
    Gif(Box<GifExport>),
}

impl ExportTask {
    pub(super) fn progress(&self) -> Progress {
        match self {
            Self::Raster(task) => task.progress(),
            Self::Gif(export) => export.task.progress(),
        }
    }

    pub(super) fn drain_chunks(&mut self) -> Result<Vec<Vec<u8>>, PngTaskError> {
        match self {
            Self::Raster(task) => task.drain_chunks(),
            Self::Gif(export) => export.task.drain_chunks(),
        }
    }

    pub(super) fn finish(&mut self, generation: Generation) -> Result<Vec<Vec<u8>>, PngTaskError> {
        match self {
            Self::Raster(task) => task.finish(generation),
            Self::Gif(export) => export.task.finish(generation),
        }
    }
}

/// A PNG or text export of one sampled grid at a time.
#[derive(Debug)]
pub(super) enum RasterTask {
    Png(Box<ArtworkPngTask>),
    Text(Box<ArtworkTextTask>),
}

impl RasterTask {
    fn progress(&self) -> Progress {
        match self {
            Self::Png(task) => task.progress(),
            Self::Text(task) => task.progress(),
        }
    }

    /// Output units one grid row contributes to the pixel-budget conversion. Always at least one.
    pub(super) fn grid_row_units(&self) -> u64 {
        match self {
            Self::Png(task) => task.pixels_per_grid_row(),
            Self::Text(task) => task.grid_row_units(),
        }
    }

    fn advance(&mut self, generation: Generation, rows: u64) -> Result<Progress, PngTaskError> {
        match self {
            Self::Png(task) => task.advance(generation, rows),
            Self::Text(task) => task.advance(generation, rows),
        }
    }

    fn drain_chunks(&mut self) -> Result<Vec<Vec<u8>>, PngTaskError> {
        match self {
            Self::Png(task) => task.drain_chunks(),
            Self::Text(task) => task.drain_chunks(),
        }
    }

    fn finish(&mut self, generation: Generation) -> Result<Vec<Vec<u8>>, PngTaskError> {
        match self {
            Self::Png(task) => task.finish(generation),
            Self::Text(task) => task.finish(generation),
        }
    }
}

/// The sampling settings every frame of one GIF export shares.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct FrameSampling {
    pub(super) cell_size: u8,
    pub(super) cell_height: u32,
    pub(super) sampling: Sampling,
}

/// A GIF export across all frames. It holds the frame's sampling task only while that frame
/// samples, and the frame's grid only while it renders; both are released before the next frame.
#[derive(Debug)]
pub(super) struct GifExport {
    task: ArtworkGifTask,
    frame: FrameSampling,
    sampling: Option<Box<SamplingTask>>,
}

impl GifExport {
    pub(super) fn new(task: ArtworkGifTask, frame: FrameSampling) -> Self {
        Self {
            task,
            frame,
            sampling: None,
        }
    }

    /// Spends `budget` on the awaited frame: its sampling when no grid is held yet, otherwise
    /// its rendering and encoding. Setup of a new frame runs at the start of the next call.
    fn step(
        &mut self,
        frames: &mut [FrameSlot],
        generation: Generation,
        budget: u64,
    ) -> Result<Progress, SessionError> {
        if !self.task.needs_grid() {
            self.task.advance(generation, budget)?;
            return Ok(self.task.progress());
        }
        let index = self.task.frame_index();
        let count = frames.len();
        let slot = frames.get_mut(index).ok_or(SessionError::FrameOutOfRange {
            frame: index,
            count,
        })?;
        let mut sampling = match self.sampling.take() {
            Some(task) => task,
            None => match slot.sampler.prepare(
                self.frame.cell_size,
                self.frame.cell_height,
                self.frame.sampling,
                generation,
            )? {
                SamplingPlan::Cached(grid) => {
                    self.task.begin_frame(generation, grid)?;
                    return Ok(self.task.progress());
                }
                SamplingPlan::Task(task) => task,
            },
        };
        match sampling.advance(budget, generation)? {
            SamplingAdvance::InProgress(_) => self.sampling = Some(sampling),
            SamplingAdvance::Completed(_) => {
                let grid = slot.sampler.complete(*sampling, generation)?;
                self.task.begin_frame(generation, grid)?;
            }
        }
        Ok(self.task.progress())
    }
}

/// Invariant: `frame` indexes the owning session's frame table, and the job's sampler
/// is that frame's sampler, so a sampling task always completes against its own source.
/// A GIF export moves `frame` forward as each frame is encoded.
#[derive(Debug)]
pub(super) struct Job {
    pub(super) generation: Generation,
    pub(super) kind: JobKind,
    pub(super) frame: usize,
    pub(super) phase: Phase,
}

/// Export gets its own renderer so the export path never shares preview glyph state.
pub(super) fn export_phase(
    grid: Arc<SampleGrid>,
    options: RenderOptions,
    generation: Generation,
    output: OutputKind,
) -> Result<Phase, SessionError> {
    let renderer = Renderer::new()?;
    let task = match output {
        OutputKind::Png => RasterTask::Png(Box::new(ArtworkPngTask::new(
            grid,
            renderer,
            options,
            generation,
            EXPORT_LIMITS,
        )?)),
        OutputKind::Text => RasterTask::Text(Box::new(ArtworkTextTask::new(
            grid,
            renderer,
            options,
            generation,
            EXPORT_LIMITS,
        )?)),
        OutputKind::Gif => {
            return Err(SessionError::Contract(ContractError::InvalidSettings(
                "gif export has its own job path",
            )))
        }
    };
    Ok(Phase::Encoding(Box::new(ExportTask::Raster(task))))
}

fn sampled_phase(
    grid: Arc<SampleGrid>,
    renderer: &mut Renderer,
    options: RenderOptions,
    kind: JobKind,
    output: OutputKind,
    generation: Generation,
) -> Result<Phase, SessionError> {
    match kind {
        JobKind::Preview => renderer
            .preview_task(grid, options, generation)
            .map(|preview| Phase::Preview(Box::new(preview)))
            .map_err(SessionError::from),
        JobKind::Export => export_phase(grid, options, generation, output),
    }
}

fn step_sampling(
    mut task: Box<SamplingTask>,
    sampler: &mut Sampler,
    renderer: &mut Renderer,
    (options, kind, output): (RenderOptions, JobKind, OutputKind),
    generation: Generation,
    budget: u64,
) -> Phase {
    match task.advance(budget, generation) {
        Ok(SamplingAdvance::InProgress(_)) => Phase::Sampling(task, options, kind, output),
        Ok(SamplingAdvance::Completed(_)) => {
            let outcome = sampler
                .complete(*task, generation)
                .map_err(SessionError::from)
                .and_then(|grid| sampled_phase(grid, renderer, options, kind, output, generation));
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

/// Converts the pixel budget into whole grid rows, at least one row, then encodes them.
/// The call that completes the last row only marks the export ready; the trailer is
/// written by `finish`.
fn step_raster(mut task: RasterTask, generation: Generation, budget: u64) -> Phase {
    let rows = budget
        .checked_div(task.grid_row_units())
        .unwrap_or(0)
        .max(1);
    match task.advance(generation, rows) {
        Ok(progress) if progress.completed() == progress.total() => {
            Phase::ExportReady(Box::new(ExportTask::Raster(task)))
        }
        Ok(_) => Phase::Encoding(Box::new(ExportTask::Raster(task))),
        Err(error) => Phase::Failed(error.into()),
    }
}

fn step_gif(
    mut export: Box<GifExport>,
    frames: &mut [FrameSlot],
    generation: Generation,
    budget: u64,
) -> Phase {
    match export.step(frames, generation, budget) {
        Ok(progress) if progress.completed() == progress.total() => {
            Phase::ExportReady(Box::new(ExportTask::Gif(export)))
        }
        Ok(_) => Phase::Encoding(Box::new(ExportTask::Gif(export))),
        Err(error) => Phase::Failed(error),
    }
}

fn step_encoding(
    task: ExportTask,
    frames: &mut [FrameSlot],
    generation: Generation,
    budget: u64,
) -> Phase {
    match task {
        ExportTask::Raster(raster) => step_raster(raster, generation, budget),
        ExportTask::Gif(export) => step_gif(export, frames, generation, budget),
    }
}

impl Job {
    /// Spends `budget` on the current stage. `frames` is the session's frame table; the job
    /// samples `frames[self.frame]`, and a GIF export moves `self.frame` as each frame completes.
    pub(super) fn advance(
        &mut self,
        frames: &mut [FrameSlot],
        renderer: &mut Renderer,
        budget: u64,
    ) -> Result<Stage, SessionError> {
        let generation = self.generation;
        let index = self.frame;
        let phase = mem::replace(&mut self.phase, Phase::Failed(SessionError::Interrupted));
        self.phase = match phase {
            Phase::Sampling(task, options, kind, output) => match frames.get_mut(index) {
                Some(slot) => step_sampling(
                    task,
                    &mut slot.sampler,
                    renderer,
                    (options, kind, output),
                    generation,
                    budget,
                ),
                None => Phase::Failed(SessionError::FrameOutOfRange {
                    frame: index,
                    count: frames.len(),
                }),
            },
            Phase::Preview(task) => step_preview(task, generation, budget),
            Phase::Original(task) => step_original(task, generation, budget),
            Phase::Encoding(task) => step_encoding(*task, frames, generation, budget),
            done @ (Phase::Ready(_)
            | Phase::OriginalReady(_)
            | Phase::ExportReady(_)
            | Phase::Failed(_)) => done,
        };
        if let Phase::Encoding(task) = &self.phase {
            if let ExportTask::Gif(export) = task.as_ref() {
                self.frame = export.task.frame_index();
            }
        }
        match &self.phase {
            Phase::Failed(error) => Err(error.clone()),
            phase => Ok(phase.stage()),
        }
    }
}
