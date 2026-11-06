//! Shared numeric bounds that the core validates against and the UI reads.

/// Smallest accepted source-cell and output-cell size in pixels.
pub const MIN_CELL_SIZE: u8 = 5;
/// Largest accepted source-cell and output-cell size in pixels.
pub const MAX_CELL_SIZE: u8 = 30;
/// Smallest accepted font size in pixels.
pub const MIN_FONT_SIZE: u8 = 8;
/// Largest accepted font size in pixels.
pub const MAX_FONT_SIZE: u8 = 30;
/// Smallest luminance threshold; every `u8` value is a valid threshold.
pub const MIN_THRESHOLD: u8 = u8::MIN;
/// Largest luminance threshold; every `u8` value is a valid threshold.
pub const MAX_THRESHOLD: u8 = u8::MAX;
/// Smallest contrast adjustment in percent. `-100` flattens every luminance to mid-grey.
pub const MIN_CONTRAST: i8 = -100;
/// Largest contrast adjustment in percent. `100` doubles distance from mid-grey.
pub const MAX_CONTRAST: i8 = 100;
/// Largest source-cell height in pixels accepted for average sampling and aspect geometry.
///
/// The bound keeps each cell's accumulation and rounded line-height arithmetic
/// far below `u32` and `u16` limits.
pub const MAX_SOURCE_CELL_HEIGHT: u16 = 1024;
/// Maximum number of printable ASCII entries in a custom ramp.
pub const MAX_RAMP_LENGTH: usize = 64;
