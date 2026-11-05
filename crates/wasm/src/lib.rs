//! WASM bindings exposing one incremental, cancellable preview session.
//!
//! Source pixels, sample grids, glyph masks and raster buffers stay private to
//! the session. Sources enter as RGBA8 or as encoded BMP, GIF, JPEG, PNG, TIFF
//! and WebP bytes. Every fallible export throws an `Error` whose `code` property
//! is a stable machine-readable string; binding failures are never reported as
//! traps, but allocations made inside codecs are best effort.

mod error;
mod numbers;
mod session;

use js_sys::{Error as JsError, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;

use crate::error::{BindingError, ErrorCode};
use crate::numbers::js_count;
use crate::session::{EncodedPlan, Frame, Session, SourcePlan, Stage};

fn to_js(error: &BindingError) -> JsValue {
    let js_error = JsError::new(error.message());
    js_error.set_name("ImageToBinaryError");
    let value = JsValue::from(js_error);
    let _ = Reflect::set(
        &value,
        &JsValue::from_str("code"),
        &JsValue::from_str(error.code().as_str()),
    );
    value
}

fn lift<T>(result: Result<T, BindingError>) -> Result<T, JsValue> {
    result.map_err(|error| to_js(&error))
}

fn number(value: &JsValue) -> f64 {
    value.as_f64().unwrap_or(f64::NAN)
}

fn copy_source(
    plan: Result<SourcePlan, BindingError>,
    pixels: &Uint8Array,
) -> Result<Session, BindingError> {
    let plan = plan?;
    if !pixels.is_instance_of::<Uint8Array>() {
        return Err(BindingError::new(
            ErrorCode::InvalidArgument,
            "pixels must be a Uint8Array",
        ));
    }
    plan.check_len(usize::try_from(pixels.length()).unwrap_or(usize::MAX))?;
    let mut buffer = plan.allocate()?;
    pixels.copy_to(&mut buffer);
    plan.into_session(buffer)
}

fn build_session(
    plan: Result<SourcePlan, BindingError>,
    pixels: &Uint8Array,
) -> Result<ImageSession, JsValue> {
    lift(copy_source(plan, pixels)).map(|inner| ImageSession { inner })
}

fn copy_encoded(bytes: &Uint8Array) -> Result<Session, BindingError> {
    if !bytes.is_instance_of::<Uint8Array>() {
        return Err(BindingError::new(
            ErrorCode::InvalidArgument,
            "bytes must be a Uint8Array",
        ));
    }
    let plan = EncodedPlan::new(usize::try_from(bytes.length()).unwrap_or(usize::MAX))?;
    let mut buffer = plan.allocate()?;
    bytes.copy_to(&mut buffer);
    EncodedPlan::decode_session(buffer)
}

/// Position of the current job in the sampling then preview pipeline.
#[wasm_bindgen]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobStage {
    Idle = 0,
    Sampling = 1,
    Preview = 2,
    Ready = 3,
    Failed = 4,
}

impl From<Stage> for JobStage {
    fn from(stage: Stage) -> Self {
        match stage {
            Stage::Idle => Self::Idle,
            Stage::Sampling => Self::Sampling,
            Stage::Preview => Self::Preview,
            Stage::Ready => Self::Ready,
            Stage::Failed => Self::Failed,
        }
    }
}

/// A finished preview. Pixels can be taken exactly once as an owned copy.
#[wasm_bindgen]
pub struct PreviewFrame {
    inner: Frame,
}

#[wasm_bindgen]
impl PreviewFrame {
    /// Frame width in pixels.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn width(&self) -> u32 {
        self.inner.width()
    }

    /// Frame height in pixels.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn height(&self) -> u32 {
        self.inner.height()
    }

    /// Returns the RGBA8 pixels as a fresh `Uint8Array` owned by JS, never a
    /// view into WASM memory, and releases the Rust copy.
    ///
    /// # Errors
    ///
    /// Throws `frame_consumed` when the pixels were already taken.
    #[wasm_bindgen(js_name = takePixels)]
    pub fn take_pixels(&mut self) -> Result<Vec<u8>, JsValue> {
        lift(self.inner.take_pixels())
    }
}

/// One normalized RGBA8 source with its sample cache, renderer and at most one
/// job. Numeric arguments must be JS numbers holding safe integers.
#[wasm_bindgen]
pub struct ImageSession {
    inner: Session,
}

#[wasm_bindgen]
impl ImageSession {
    /// Validates dimensions, length and the 50 MP ceiling, then copies the
    /// orientation-normalized RGBA8 `pixels` once into WASM memory.
    ///
    /// # Errors
    ///
    /// Throws `invalid_argument`, `invalid_dimensions` or `resource_limit_exceeded`.
    #[wasm_bindgen(js_name = fromRgba)]
    pub fn from_rgba(
        #[wasm_bindgen(unchecked_param_type = "number")] width: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "number")] height: &JsValue,
        pixels: &Uint8Array,
    ) -> Result<ImageSession, JsValue> {
        build_session(SourcePlan::new(number(width), number(height), None), pixels)
    }

    /// Like `fromRgba` with a lower caller pixel ceiling (at most 50 MP).
    ///
    /// # Errors
    ///
    /// Throws the `fromRgba` errors, or `invalid_settings` for a bad ceiling.
    #[wasm_bindgen(js_name = fromRgbaWithMaxPixels)]
    pub fn from_rgba_with_max_pixels(
        #[wasm_bindgen(unchecked_param_type = "number")] width: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "number")] height: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "number")] max_pixels: &JsValue,
        pixels: &Uint8Array,
    ) -> Result<ImageSession, JsValue> {
        build_session(
            SourcePlan::new(number(width), number(height), Some(number(max_pixels))),
            pixels,
        )
    }

    /// Decodes BMP, GIF, JPEG, PNG, TIFF or WebP bytes with the Rust decoder and
    /// builds a session from the orientation-normalized RGBA8 result.
    ///
    /// The runtime type and the 256 MiB length ceiling are checked before any
    /// byte is copied. `bytes` is then copied once into fallibly reserved WASM
    /// memory; the caller's array is never retained or mutated and the copy is
    /// freed before the session is returned. Decoded pixels are moved into the
    /// session, not cloned. GIF and TIFF yield their first frame or page.
    ///
    /// Decoding is synchronous and runs to completion: it cannot be cancelled or
    /// preempted mid-call. The 50 MP and 1 GiB decoded-byte ceilings are checked
    /// from headers before pixels are allocated, but allocations made inside
    /// codecs are best effort and may still trap on out-of-memory.
    ///
    /// # Errors
    ///
    /// Throws `invalid_argument`, `encoded_too_large`, `reservation_failed`,
    /// `unsupported_format`, `decode_failed`, `invalid_dimensions` or
    /// `resource_limit_exceeded`.
    #[wasm_bindgen(js_name = fromEncoded)]
    pub fn from_encoded(bytes: &Uint8Array) -> Result<ImageSession, JsValue> {
        lift(copy_encoded(bytes)).map(|inner| ImageSession { inner })
    }

    /// Immutable post-orientation source width in pixels.
    ///
    /// # Errors
    ///
    /// Throws `disposed`.
    #[wasm_bindgen(getter, js_name = sourceWidth)]
    pub fn source_width(&self) -> Result<u32, JsValue> {
        lift(self.inner.source_metadata()).map(|metadata| metadata.dimensions().width())
    }

    /// Immutable post-orientation source height in pixels.
    ///
    /// # Errors
    ///
    /// Throws `disposed`.
    #[wasm_bindgen(getter, js_name = sourceHeight)]
    pub fn source_height(&self) -> Result<u32, JsValue> {
        lift(self.inner.source_metadata()).map(|metadata| metadata.dimensions().height())
    }

    /// Normalized source channel count (always 4, RGBA8).
    ///
    /// # Errors
    ///
    /// Throws `disposed`.
    #[wasm_bindgen(getter, js_name = sourceChannels)]
    pub fn source_channels(&self) -> Result<u8, JsValue> {
        lift(self.inner.source_metadata()).map(image_to_binary_core::SourceMetadata::channels)
    }

    /// Starts a job superseding any current one. `generation` must exceed every
    /// earlier accepted generation (zero is valid first); `sampling` is
    /// `"classic"` or `"average"`. Allocation and font setup are synchronous.
    ///
    /// # Errors
    ///
    /// Throws `invalid_argument`, `invalid_settings`, `stale_generation`,
    /// `disposed` or a resource error; invalid or stale starts keep current state.
    #[wasm_bindgen(js_name = startPreview)]
    pub fn start_preview(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "number")] generation: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "number")] cell_size: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "number")] font_size: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "number")] threshold: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "\"classic\" | \"average\"")] sampling: &JsValue,
    ) -> Result<JobStage, JsValue> {
        let sampling = sampling.as_string();
        lift(self.inner.start_preview(
            number(generation),
            number(cell_size),
            number(font_size),
            number(threshold),
            sampling.as_deref(),
        ))
        .map(JobStage::from)
    }

    /// Starts a bounded original-image preview superseding any current job.
    ///
    /// The job reports `Preview` then `Ready` and shares `advance`, `cancel`, `finish`,
    /// `stage` and the progress getters with artwork jobs. `generation` must exceed
    /// every earlier accepted generation of either kind. The frame is the normalized
    /// source downscaled by nearest-center sampling to a longest edge of at most 1600
    /// pixels, never upscaled; source pixels are never exposed. Output allocation is
    /// synchronous.
    ///
    /// # Errors
    ///
    /// Throws `invalid_argument`, `stale_generation`, `disposed` or a resource error;
    /// invalid or stale starts keep the current job.
    #[wasm_bindgen(js_name = startOriginalPreview)]
    pub fn start_original_preview(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "number")] generation: &JsValue,
    ) -> Result<JobStage, JsValue> {
        lift(self.inner.start_original_preview(number(generation))).map(JobStage::from)
    }

    /// Returns the current job stage, `Idle` when there is none.
    ///
    /// # Errors
    ///
    /// Throws `disposed`.
    pub fn stage(&self) -> Result<JobStage, JsValue> {
        lift(self.inner.stage()).map(JobStage::from)
    }

    /// Work units completed in the current stage.
    ///
    /// # Errors
    ///
    /// Throws `disposed`.
    #[wasm_bindgen(js_name = progressCompleted)]
    pub fn progress_completed(&self) -> Result<f64, JsValue> {
        lift(self.inner.progress()).map(|(done, _)| js_count(done))
    }

    /// Work units in the current stage.
    ///
    /// # Errors
    ///
    /// Throws `disposed`.
    #[wasm_bindgen(js_name = progressTotal)]
    pub fn progress_total(&self) -> Result<f64, JsValue> {
        lift(self.inner.progress()).map(|(_, total)| js_count(total))
    }

    /// Performs at most `pixel_budget` units in the current stage only; the call
    /// completing a stage does its non-interruptible setup but no next-stage work.
    ///
    /// # Errors
    ///
    /// Throws `invalid_argument`, `no_job`, `stale_generation`, `cancelled`,
    /// `disposed` or a terminal job error that cannot be revived.
    pub fn advance(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "number")] generation: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "number")] pixel_budget: &JsValue,
    ) -> Result<JobStage, JsValue> {
        lift(self.inner.advance(number(generation), number(pixel_budget))).map(JobStage::from)
    }

    /// Cancels the job and releases its partial work. Repeating it is a no-op.
    ///
    /// # Errors
    ///
    /// Throws `invalid_argument`, `no_job`, `stale_generation` or `disposed`.
    pub fn cancel(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "number")] generation: &JsValue,
    ) -> Result<(), JsValue> {
        lift(self.inner.cancel(number(generation)))
    }

    /// Returns the finished frame once, and only when the job is `Ready`.
    ///
    /// # Errors
    ///
    /// Throws `job_incomplete`, `no_job`, `stale_generation`, `cancelled` or `disposed`.
    pub fn finish(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "number")] generation: &JsValue,
    ) -> Result<PreviewFrame, JsValue> {
        lift(self.inner.finish(number(generation))).map(|inner| PreviewFrame { inner })
    }

    /// Releases the source, caches, renderer and job. Idempotent.
    pub fn dispose(&mut self) {
        self.inner.dispose();
    }
}
