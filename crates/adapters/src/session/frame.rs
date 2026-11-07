//! A finished preview frame. Its pixels can be taken exactly once.

use image_to_binary_processing::RenderedFrame;

use super::error::SessionError;

/// A finished preview frame. The RGBA8 pixels are owned here until taken once.
///
/// Invariant: `width * height * 4` equals the length of the pixels until they are taken.
#[derive(Debug)]
pub struct Frame {
    width: u32,
    height: u32,
    pixels: Option<Vec<u8>>,
}

impl Frame {
    pub(super) fn new(frame: RenderedFrame) -> Self {
        Self {
            width: frame.width(),
            height: frame.height(),
            pixels: Some(frame.into_pixels()),
        }
    }

    /// Returns the frame width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the frame height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Moves the RGBA8 pixels out and leaves the frame empty.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::FrameConsumed`] when the pixels were already taken.
    pub fn take_pixels(&mut self) -> Result<Vec<u8>, SessionError> {
        self.pixels.take().ok_or(SessionError::FrameConsumed)
    }
}
