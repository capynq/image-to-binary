//! Opaque adapter-session lifecycle shared by future native and browser shells.

use image_to_binary_core::ContractError;
use image_to_binary_processing::Job;

/// Adapter-owned session state; resources are released when the session drops.
#[derive(Debug)]
pub struct Session {
    job: Option<Job>,
    disposed: bool,
}

impl Session {
    /// Creates an active session around one processing job.
    #[must_use]
    pub fn new(job: Job) -> Self {
        Self {
            job: Some(job),
            disposed: false,
        }
    }
    /// Starts the owned job while retaining adapter ownership.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidSettings`] when the session was
    /// disposed or no longer owns a job.
    pub fn start(&mut self) -> Result<(), ContractError> {
        if self.disposed {
            return Err(ContractError::InvalidSettings("disposed session"));
        }
        self.job
            .as_mut()
            .ok_or(ContractError::InvalidSettings("missing job"))?
            .start()
    }
    /// Cancels and releases the owned job.
    pub fn dispose(&mut self) {
        if let Some(job) = self.job.as_mut() {
            job.cancel();
        }
        self.job = None;
        self.disposed = true;
    }
    /// Returns whether the session has released its resources.
    #[must_use]
    pub const fn is_disposed(&self) -> bool {
        self.disposed
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.dispose();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image_to_binary_core::{Dimensions, Generation, Sampling, Settings, SourceMetadata};
    use image_to_binary_processing::Request;

    #[test]
    fn dispose_is_idempotent_and_blocks_start() {
        let metadata = SourceMetadata::new(Dimensions::new(1, 1).unwrap(), 1).unwrap();
        let settings = Settings::new(5, 128, Sampling::Classic, "01").unwrap();
        let request = Request::new(metadata, settings, Generation::new(1));
        let job = Job::new(request, 1).unwrap();
        let mut session = Session::new(job);
        session.dispose();
        session.dispose();
        assert!(session.is_disposed());
        assert!(session.start().is_err());
    }
}
