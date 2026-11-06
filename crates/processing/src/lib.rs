//! Normalized image decoding, deterministic cell sampling, and processing lifecycle contracts.

mod decode;
mod original;
mod renderer;
mod sampling;

pub use decode::{decode, DecodeError, DecodeLimits, DecodedSource};
pub use original::{OriginalPreviewTask, ORIGINAL_PREVIEW_MAX_EDGE};
pub use renderer::{
    AspectGeometry, PreviewAdvance, PreviewTask, RenderError, RenderOptions, RenderedFrame,
    Renderer, RendererCacheStats, MAX_RENDER_ROW_BYTES,
};
pub use sampling::{SampleCell, SampleGrid, Sampler, SamplingAdvance, SamplingPlan, SamplingTask};

use image_to_binary_core::{
    Cancellation, ContractError, Generation, Progress, Settings, SourceMetadata,
};

/// Opaque processing request shared by native and browser adapters.
#[derive(Clone, Debug)]
pub struct Request {
    metadata: SourceMetadata,
    settings: Settings,
    generation: Generation,
}

impl Request {
    /// Creates a request with immutable source and rendering contracts.
    #[must_use]
    pub const fn new(metadata: SourceMetadata, settings: Settings, generation: Generation) -> Self {
        Self {
            metadata,
            settings,
            generation,
        }
    }
    /// Returns source metadata.
    #[must_use]
    pub const fn metadata(&self) -> SourceMetadata {
        self.metadata
    }
    /// Returns deterministic settings.
    #[must_use]
    pub const fn settings(&self) -> &Settings {
        &self.settings
    }
    /// Returns the result generation.
    #[must_use]
    pub const fn generation(&self) -> Generation {
        self.generation
    }
}

/// Lifecycle state for one processing job.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobState {
    Pending,
    Running,
    Completed,
    Cancelled,
}

/// Single-use job state machine owned by an adapter session.
#[derive(Debug)]
pub struct Job {
    request: Request,
    state: JobState,
    progress: Progress,
    cancellation: Cancellation,
}

impl Job {
    /// Creates a pending job with a caller-defined work total.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidProgress`] for a zero total.
    pub fn new(request: Request, total: u64) -> Result<Self, ContractError> {
        Ok(Self {
            cancellation: Cancellation::new(request.generation()),
            request,
            state: JobState::Pending,
            progress: Progress::new(total)?,
        })
    }
    /// Starts a pending job.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidSettings`] when the job is not pending.
    pub fn start(&mut self) -> Result<(), ContractError> {
        if self.state != JobState::Pending {
            return Err(ContractError::InvalidSettings("job state"));
        }
        self.state = JobState::Running;
        Ok(())
    }
    /// Advances a running job and returns its cancellation status.
    ///
    /// # Errors
    ///
    /// Returns a cancellation or generation error, an invalid-state error, or
    /// a progress arithmetic/bounds error.
    pub fn advance(&mut self, amount: u64) -> Result<(), ContractError> {
        self.cancellation.check(self.request.generation())?;
        if self.state != JobState::Running {
            return Err(ContractError::InvalidSettings("job state"));
        }
        self.progress.advance(amount)
    }
    /// Marks a fully advanced job complete.
    ///
    /// # Errors
    ///
    /// Returns a cancellation or generation error, or an invalid state/progress
    /// error when work is incomplete.
    pub fn complete(&mut self) -> Result<(), ContractError> {
        self.cancellation.check(self.request.generation())?;
        if self.state != JobState::Running || self.progress.completed() != self.progress.total() {
            return Err(ContractError::InvalidProgress);
        }
        self.state = JobState::Completed;
        Ok(())
    }
    /// Requests cancellation and transitions the job to cancelled.
    pub fn cancel(&mut self) {
        self.cancellation.cancel();
        self.state = JobState::Cancelled;
    }
    /// Returns the current lifecycle state.
    #[must_use]
    pub const fn state(&self) -> JobState {
        self.state
    }
    /// Returns a snapshot of completed and total work units.
    #[must_use]
    pub const fn progress(&self) -> Progress {
        self.progress
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image_to_binary_core::{Dimensions, Sampling};

    fn request() -> Request {
        let metadata = SourceMetadata::new(Dimensions::new(1, 1).unwrap(), 1).unwrap();
        let settings = Settings::new(5, 128, Sampling::Classic, "01").unwrap();
        Request::new(metadata, settings, Generation::new(0))
    }

    #[test]
    fn job_exposes_progress_and_rejects_invalid_lifecycle() {
        let mut job = Job::new(request(), 2).unwrap();
        assert_eq!(job.progress().completed(), 0);
        assert!(job.complete().is_err());
        job.start().unwrap();
        job.advance(1).unwrap();
        assert_eq!(job.progress().completed(), 1);
        assert!(job.complete().is_err());
        job.advance(1).unwrap();
        job.complete().unwrap();
        assert_eq!(job.state(), JobState::Completed);
        assert!(job.start().is_err());
    }
}
