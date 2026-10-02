//! Stable, dependency-free contracts for the image-to-binary engine.

mod cancellation;
mod error;
mod geometry;
mod output;
mod progress;
mod settings;
mod source;

pub use cancellation::{Cancellation, Generation};
pub use error::ContractError;
pub use geometry::{Dimensions, ResourceLimits};
pub use output::{Output, OutputFormat};
pub use progress::Progress;
pub use settings::{Sampling, Settings};
pub use source::SourceMetadata;
