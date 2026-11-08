//! The single job slot: sampling, then preview or encoding, then ready or failed.
//!
//! Each `advance` spends its pixel budget in the current stage only. The call that
//! completes a stage performs that stage's non-interruptible setup and no next-stage work.

use std::mem;
use std::sync::Arc;

use image_to_binary_core::{Generation, Progress};
use image_to_binary_processing::{
    OriginalPreviewTask, PreviewAdvance, PreviewTask, RenderOptions, Renderer, SampleGrid, Sampler,
    SamplingAdvance, SamplingTask,
};

use super::error::{JobKind, SessionError};
use super::limits::EXPORT_LIMITS;
use crate::{ArtworkPngTask, ArtworkTextTask, PngTaskError};

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
    /// Encoding export rows; progress counts grid rows.
    Encoding,
    /// The job finished. A preview can be taken once; an export can be finished.
    Ready,
    /// Terminal failure. The error is returned until a newer start replaces the job.
    Failed,
}

/// The file encoding an export job produces. `Png` is the default; `Text` produces
/// UTF-8 grid rows over the same sampled grid and render options.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OutputKind {
    /// A PNG image.
    #[default]
    Png,
    /// UTF-8 text, one grid row per line, each line ending in LF.
    Text,
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

/// One export encoder, chosen once when the job reaches encoding. Both variants share
/// the same grid, render options, limits and lifecycle, so they differ only in row encoding.
#[derive(Debug)]
pub(super) enum ExportTask {
    Png(Box<ArtworkPngTask>),
    Text(Box<ArtworkTextTask>),
}

impl ExportTask {
    pub(super) fn progress(&self) -> Progress {
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

    pub(super) fn advance(
        &mut self,
        generation: Generation,
        rows: u64,
    ) -> Result<Progress, PngTaskError> {
        match self {
            Self::Png(task) => task.advance(generation, rows),
            Self::Text(task) => task.advance(generation, rows),
        }
    }

    pub(super) fn drain_chunks(&mut self) -> Result<Vec<Vec<u8>>, PngTaskError> {
        match self {
            Self::Png(task) => task.drain_chunks(),
            Self::Text(task) => task.drain_chunks(),
        }
    }

    pub(super) fn finish(&mut self, generation: Generation) -> Result<Vec<Vec<u8>>, PngTaskError> {
        match self {
            Self::Png(task) => task.finish(generation),
            Self::Text(task) => task.finish(generation),
        }
    }
}

/// Invariant: `frame` indexes the owning session's frame table, and the job's sampler
/// is that frame's sampler, so a sampling task always completes against its own source.
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
        OutputKind::Png => ExportTask::Png(Box::new(ArtworkPngTask::new(
            grid,
            renderer,
            options,
            generation,
            EXPORT_LIMITS,
        )?)),
        OutputKind::Text => ExportTask::Text(Box::new(ArtworkTextTask::new(
            grid,
            renderer,
            options,
            generation,
            EXPORT_LIMITS,
        )?)),
    };
    Ok(Phase::Encoding(Box::new(task)))
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
fn step_encoding(mut task: Box<ExportTask>, generation: Generation, budget: u64) -> Phase {
    let rows = budget
        .checked_div(task.grid_row_units())
        .unwrap_or(0)
        .max(1);
    match task.advance(generation, rows) {
        Ok(progress) if progress.completed() == progress.total() => Phase::ExportReady(task),
        Ok(_) => Phase::Encoding(task),
        Err(error) => Phase::Failed(error.into()),
    }
}

impl Job {
    pub(super) fn advance(
        &mut self,
        sampler: &mut Sampler,
        renderer: &mut Renderer,
        budget: u64,
    ) -> Result<Stage, SessionError> {
        let generation = self.generation;
        let phase = mem::replace(&mut self.phase, Phase::Failed(SessionError::Interrupted));
        self.phase = match phase {
            Phase::Sampling(task, options, kind, output) => step_sampling(
                task,
                sampler,
                renderer,
                (options, kind, output),
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
