//! Resource ceilings shared by every front end that drives a [`super::Session`].
//!
//! These are the only definitions. The WASM shell and the native shell both read them, so
//! a change here reaches both; the native cross-check test reads the browser's pixel budget
//! to keep the two copies equal.

use crate::PngTaskLimits;

/// Largest source width times height accepted by the session, in pixels (50 MP). It equals the
/// core default pixel ceiling and the browser's `MAX_SOURCE_PIXELS`.
pub const MAX_SOURCE_PIXELS: u64 = 50_000_000;

/// Largest encoded input accepted by the session, in bytes (256 MiB).
pub const MAX_ENCODED_BYTES: u64 = 256 * 1024 * 1024;
/// Largest decoded RGBA working set accepted for one source, in bytes (1 GiB).
pub const MAX_DECODED_BYTES: u64 = 1024 * 1024 * 1024;

/// Ceiling for one exported PNG, header and trailer included. It equals the encoded
/// input ceiling so an export never produces more bytes than the session accepts as input.
pub const EXPORT_MAX_ENCODED_BYTES: u64 = MAX_ENCODED_BYTES;

/// Size of each completed chunk handed to a boundary. One mebibyte bounds every copy
/// while keeping a maximal export to at most 256 chunks.
pub const EXPORT_CHUNK_BYTES: usize = 1024 * 1024;

/// Pending chunk storage ceiling. It equals the encoded ceiling so one advance can emit the
/// whole output without failing merely because the caller has not drained yet.
pub const EXPORT_MAX_PENDING_BYTES: u64 = EXPORT_MAX_ENCODED_BYTES;

/// Limits handed to every PNG export task created by the session.
pub const EXPORT_LIMITS: PngTaskLimits = PngTaskLimits::new(
    EXPORT_MAX_ENCODED_BYTES,
    EXPORT_CHUNK_BYTES,
    EXPORT_MAX_PENDING_BYTES,
);

/// Pixels of stage work each shell spends per advance call for previews and exports.
/// The browser copy in `frontend/src/processing/workerHost.ts` must equal this value.
pub const DEFAULT_PIXEL_BUDGET: u64 = 131_072;

#[cfg(test)]
mod tests {
    use image_to_binary_core::ResourceLimits;

    use super::MAX_SOURCE_PIXELS;

    #[test]
    fn the_pixel_ceiling_equals_the_core_default() {
        assert_eq!(MAX_SOURCE_PIXELS, ResourceLimits::default().max_pixels());
    }
}
