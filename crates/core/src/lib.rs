//! Stable, dependency-free contracts for the image-to-binary engine.

mod bounds;
mod cancellation;
mod error;
mod geometry;
mod output;
mod progress;
mod ramp;
mod settings;
mod source;

pub use bounds::{
    MAX_CELL_SIZE, MAX_CONTRAST, MAX_FONT_SIZE, MAX_RAMP_LENGTH, MAX_SOURCE_CELL_HEIGHT,
    MAX_THRESHOLD, MIN_CELL_SIZE, MIN_CONTRAST, MIN_FONT_SIZE, MIN_THRESHOLD,
};
pub use cancellation::{Cancellation, Generation};
pub use error::ContractError;
pub use geometry::{Dimensions, ResourceLimits};
pub use output::{Output, OutputFormat};
pub use progress::Progress;
pub use ramp::Ramp;
pub use settings::{RenderMode, Sampling, Settings};
pub use source::SourceMetadata;
