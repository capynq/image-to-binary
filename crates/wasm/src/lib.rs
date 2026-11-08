//! WASM bindings exposing one incremental, cancellable preview or PNG export session.
//!
//! Source pixels, sample grids, glyph masks and raster buffers stay private to
//! the session. Sources enter as RGBA8 or as encoded BMP, GIF, JPEG, PNG, TIFF
//! and WebP bytes. Every fallible export throws an `Error` whose `code` property
//! is a stable machine-readable string; binding failures are never reported as
//! traps, but allocations made inside codecs are best effort.

mod error;
mod numbers;

use image_to_binary_adapters::session::args::{self, BoolArg, StyleArgs, TextArg};
use image_to_binary_adapters::session::{
    EncodedPlan, Frame, JobKind, LoopCount, Session, SourcePlan, Stage,
};
use image_to_binary_core::Generation;
use js_sys::{Error as JsError, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;

use crate::error::{BindingError, ErrorCode};
use crate::numbers::js_count;

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

fn lift<T, E: Into<BindingError>>(result: Result<T, E>) -> Result<T, JsValue> {
    result.map_err(|error| to_js(&error.into()))
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
    plan.check_len(usize::try_from(pixels.length()).unwrap_or(usize::MAX))
        .map_err(BindingError::from)?;
    let mut buffer = plan.allocate().map_err(BindingError::from)?;
    pixels.copy_to(&mut buffer);
    plan.into_session(buffer).map_err(BindingError::from)
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
    let plan = EncodedPlan::new(usize::try_from(bytes.length()).unwrap_or(usize::MAX))
        .map_err(BindingError::from)?;
    let mut buffer = plan.allocate().map_err(BindingError::from)?;
    bytes.copy_to(&mut buffer);
    EncodedPlan::decode_animated_session(buffer).map_err(BindingError::from)
}

/// Checks the session is live before any argument is converted, so disposal wins over
/// argument errors as the browser contract requires.
fn generation_of(session: &Session, value: &JsValue) -> Result<Generation, BindingError> {
    session.ensure_live().map_err(BindingError::from)?;
    args::generation(number(value)).map_err(BindingError::from)
}

/// The optional frame index. `undefined` and `null` select frame `0`, so existing callers
/// that omit the trailing argument keep the first frame.
fn frame_of(value: &JsValue) -> Result<usize, BindingError> {
    if value.is_undefined() || value.is_null() {
        Ok(0)
    } else {
        args::frame_index(number(value)).map_err(BindingError::from)
    }
}

/// An optional text argument owned for the duration of one call. `undefined` and `null` are
/// absent, so existing callers that omit the trailing arguments keep the classic path.
enum OwnedText {
    Absent,
    Text(String),
    NotText,
}

impl OwnedText {
    fn of(value: &JsValue) -> Self {
        if value.is_undefined() || value.is_null() {
            Self::Absent
        } else {
            value.as_string().map_or(Self::NotText, Self::Text)
        }
    }

    fn arg(&self) -> TextArg<'_> {
        match self {
            Self::Absent => TextArg::Absent,
            Self::Text(text) => TextArg::Text(text),
            Self::NotText => TextArg::NotText,
        }
    }
}

/// Optional style values as JS passed them, read after the mode and ramp. `undefined` and `null`
/// are absent, so callers that omit every trailing argument keep the classic output.
struct StyleInputs<'a> {
    foreground: &'a JsValue,
    background: &'a JsValue,
    color_mode: &'a JsValue,
    contrast: &'a JsValue,
    invert: &'a JsValue,
}

impl StyleInputs<'_> {
    fn owned(&self) -> OwnedStyle {
        OwnedStyle {
            foreground: OwnedText::of(self.foreground),
            background: OwnedText::of(self.background),
            color_mode: OwnedText::of(self.color_mode),
            contrast: if self.contrast.is_undefined() || self.contrast.is_null() {
                None
            } else {
                Some(number(self.contrast))
            },
            invert: if self.invert.is_undefined() || self.invert.is_null() {
                BoolArg::Absent
            } else {
                self.invert
                    .as_bool()
                    .map_or(BoolArg::NotBool, BoolArg::Value)
            },
        }
    }
}

/// Owned copy of [`StyleInputs`] that lends the borrowed [`StyleArgs`] for one call.
struct OwnedStyle {
    foreground: OwnedText,
    background: OwnedText,
    color_mode: OwnedText,
    contrast: Option<f64>,
    invert: BoolArg,
}

impl OwnedStyle {
    fn args(&self) -> StyleArgs<'_> {
        StyleArgs {
            foreground: self.foreground.arg(),
            background: self.background.arg(),
            color_mode: self.color_mode.arg(),
            contrast: self.contrast,
            invert: self.invert,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn start_job(
    session: &mut Session,
    kind: JobKind,
    generation: &JsValue,
    cell_size: &JsValue,
    font_size: &JsValue,
    threshold: &JsValue,
    sampling: &JsValue,
    mode: &JsValue,
    ramp: &JsValue,
    style: &StyleInputs<'_>,
    output_kind: &JsValue,
    frame: &JsValue,
) -> Result<Stage, BindingError> {
    let generation = generation_of(session, generation)?;
    let frame = frame_of(frame)?;
    let sampling = sampling.as_string();
    let mode = OwnedText::of(mode);
    let ramp = OwnedText::of(ramp);
    let style = style.owned();
    let settings = args::preview_settings_styled(
        number(cell_size),
        number(font_size),
        number(threshold),
        sampling.as_deref(),
        mode.arg(),
        ramp.arg(),
        style.args(),
    )
    .map_err(BindingError::from)?;
    match kind {
        JobKind::Preview => session.start_preview_frame(generation, frame, settings),
        JobKind::Export => {
            let output_kind = OwnedText::of(output_kind);
            let output = args::output_kind(output_kind.arg()).map_err(BindingError::from)?;
            session.start_export_frame_as(generation, frame, settings, output)
        }
    }
    .map_err(BindingError::from)
}

fn advance_job(
    session: &mut Session,
    generation: &JsValue,
    pixel_budget: &JsValue,
) -> Result<Stage, BindingError> {
    let generation = generation_of(session, generation)?;
    let budget = args::pixel_budget(number(pixel_budget)).map_err(BindingError::from)?;
    session
        .advance(generation, budget)
        .map_err(BindingError::from)
}

/// Position of the current job in the sampling, then preview or encoding,
/// pipeline. Export jobs pass through `Encoding` before `Ready`.
#[wasm_bindgen]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobStage {
    Idle = 0,
    Sampling = 1,
    Preview = 2,
    Ready = 3,
    Failed = 4,
    Encoding = 5,
}

impl From<Stage> for JobStage {
    fn from(stage: Stage) -> Self {
        match stage {
            Stage::Idle => Self::Idle,
            Stage::Sampling => Self::Sampling,
            Stage::Preview => Self::Preview,
            Stage::Encoding => Self::Encoding,
            Stage::Ready => Self::Ready,
            Stage::Failed => Self::Failed,
        }
    }
}

/// Copies each chunk into a fresh JS-owned `Uint8Array`; no view into WASM
/// memory escapes, so the arrays survive later calls, growth and disposal.
fn export_chunks(chunks: &[Vec<u8>]) -> Vec<JsValue> {
    chunks
        .iter()
        .map(|chunk| JsValue::from(Uint8Array::from(chunk.as_slice())))
        .collect()
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
        build_session(
            args::source_plan(number(width), number(height), None).map_err(BindingError::from),
            pixels,
        )
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
            args::source_plan(number(width), number(height), Some(number(max_pixels)))
                .map_err(BindingError::from),
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
    /// session, not cloned. A GIF with several frames becomes an animation session of up to
    /// 1000 frames within the decoded budget; a GIF over either limit, and a TIFF, yield their
    /// first frame or page as before.
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

    /// Number of frames: `1` for a still, the decoded frame count for an animation.
    ///
    /// # Errors
    ///
    /// Throws `disposed`.
    #[wasm_bindgen(getter, js_name = frameCount)]
    pub fn frame_count(&self) -> Result<f64, JsValue> {
        lift(self.inner.frame_count())
            .map(|count| js_count(u64::try_from(count).unwrap_or(u64::MAX)))
    }

    /// Per-frame display times in centiseconds (`Uint16Array`, one per frame, never `0` or `1`).
    /// Empty for a still.
    ///
    /// # Errors
    ///
    /// Throws `disposed`.
    #[wasm_bindgen(getter, js_name = frameDelays)]
    pub fn frame_delays(&self) -> Result<Vec<u16>, JsValue> {
        lift(self.inner.frame_delays_centiseconds()).map(<[u16]>::to_vec)
    }

    /// Repeat count after the first play: `Infinity` loops forever, and `0` plays once, which a
    /// still and an animation without a loop extension report.
    ///
    /// # Errors
    ///
    /// Throws `disposed`.
    #[wasm_bindgen(getter, js_name = loopCount)]
    pub fn loop_count(&self) -> Result<f64, JsValue> {
        lift(self.inner.loop_count()).map(|count| match count {
            LoopCount::Infinite => f64::INFINITY,
            LoopCount::Finite(times) => f64::from(times),
        })
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
    #[allow(clippy::too_many_arguments)]
    pub fn start_preview(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "number")] generation: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "number")] cell_size: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "number")] font_size: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "number")] threshold: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "\"classic\" | \"average\"")] sampling: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "\"binary\" | \"ascii\" | \"block\" | undefined")]
        mode: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "string | undefined")] ramp: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "string | undefined")] foreground: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "string | undefined")] background: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "\"monochrome\" | \"source\" | undefined")]
        color_mode: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "number | undefined")] contrast: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "boolean | undefined")] invert: &JsValue,
        #[wasm_bindgen(unchecked_optional_param_type = "number")] frame: &JsValue,
    ) -> Result<JobStage, JsValue> {
        lift(start_job(
            &mut self.inner,
            JobKind::Preview,
            generation,
            cell_size,
            font_size,
            threshold,
            sampling,
            mode,
            ramp,
            &StyleInputs {
                foreground,
                background,
                color_mode,
                contrast,
                invert,
            },
            &JsValue::UNDEFINED,
            frame,
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
        #[wasm_bindgen(unchecked_optional_param_type = "number")] frame: &JsValue,
    ) -> Result<JobStage, JsValue> {
        let generation = generation_of(&self.inner, generation);
        let frame = frame_of(frame);
        lift(generation.and_then(|generation| {
            let frame = frame?;
            self.inner
                .start_original_preview_frame(generation, frame)
                .map_err(BindingError::from)
        }))
        .map(JobStage::from)
    }

    /// Returns the current job stage, `Idle` when there is none.
    ///
    /// # Errors
    ///
    /// Throws `disposed`.
    pub fn stage(&self) -> Result<JobStage, JsValue> {
        lift(self.inner.stage()).map(JobStage::from)
    }

    /// Work completed in the current stage, in that stage's unit: sampled grid
    /// cells for classic sampling or source pixels for average sampling, output
    /// pixels while rendering a preview or original, and grid rows while an export
    /// encodes. Zero when there is no job.
    ///
    /// # Errors
    ///
    /// Throws `disposed`.
    #[wasm_bindgen(js_name = progressCompleted)]
    pub fn progress_completed(&self) -> Result<f64, JsValue> {
        lift(self.inner.progress()).map(|(done, _)| js_count(done))
    }

    /// Total work in the current stage, in the same unit as `progressCompleted`.
    ///
    /// # Errors
    ///
    /// Throws `disposed`.
    #[wasm_bindgen(js_name = progressTotal)]
    pub fn progress_total(&self) -> Result<f64, JsValue> {
        lift(self.inner.progress()).map(|(_, total)| js_count(total))
    }

    /// Spends `pixel_budget` pixels of work in the current stage only; the call
    /// completing a stage does its non-interruptible setup but no next-stage work.
    /// The budget is always pixels: an export converts it to whole grid rows of
    /// `frame width * tile height` pixels, never fewer than one row. An advance
    /// can therefore exceed a tiny budget by at most one grid row, and a huge
    /// budget finishes a stage in one call, so cancellation can take effect
    /// between any two advances.
    ///
    /// # Errors
    ///
    /// Throws `invalid_argument`, `no_job`, `stale_generation`, `cancelled`,
    /// `disposed`, `resource_limit_exceeded`, `encode_failed` or a terminal job
    /// error that cannot be revived.
    pub fn advance(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "number")] generation: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "number")] pixel_budget: &JsValue,
    ) -> Result<JobStage, JsValue> {
        lift(advance_job(&mut self.inner, generation, pixel_budget)).map(JobStage::from)
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
        lift(
            generation_of(&self.inner, generation)
                .and_then(|generation| self.inner.cancel(generation).map_err(BindingError::from)),
        )
    }

    /// Returns the finished frame once, and only when the job is a preview `Ready`.
    /// An export job is left untouched and reports `no_job`.
    ///
    /// # Errors
    ///
    /// Throws `job_incomplete`, `no_job`, `stale_generation`, `cancelled` or `disposed`.
    pub fn finish(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "number")] generation: &JsValue,
    ) -> Result<PreviewFrame, JsValue> {
        lift(
            generation_of(&self.inner, generation)
                .and_then(|generation| self.inner.finish(generation).map_err(BindingError::from)),
        )
        .map(|inner| PreviewFrame { inner })
    }

    /// Starts an export superseding any current job. It takes the same
    /// generation rule, settings and sampling as `startPreview`, so both kinds
    /// sample the same deterministic grid and share one cache. The job reports
    /// `Sampling`, `Encoding` then `Ready`; advance it with `advance`, collect
    /// bytes with `drainExport` and `finishExport`, and stop it with `cancel`.
    ///
    /// `"gif"` encodes every frame of an animation in turn and needs an animated
    /// source and frame `0` or no frame. Its `Encoding` progress counts frames.
    ///
    /// # Errors
    ///
    /// Throws `invalid_argument`, `invalid_settings`, `stale_generation`,
    /// `disposed` or a resource error; invalid or stale starts keep the current job.
    #[wasm_bindgen(js_name = startExport)]
    #[allow(clippy::too_many_arguments)]
    pub fn start_export(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "number")] generation: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "number")] cell_size: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "number")] font_size: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "number")] threshold: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "\"classic\" | \"average\"")] sampling: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "\"binary\" | \"ascii\" | \"block\" | undefined")]
        mode: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "string | undefined")] ramp: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "string | undefined")] foreground: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "string | undefined")] background: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "\"monochrome\" | \"source\" | undefined")]
        color_mode: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "number | undefined")] contrast: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "boolean | undefined")] invert: &JsValue,
        #[wasm_bindgen(unchecked_param_type = "\"png\" | \"text\" | \"gif\" | undefined")]
        output_kind: &JsValue,
        #[wasm_bindgen(unchecked_optional_param_type = "number")] frame: &JsValue,
    ) -> Result<JobStage, JsValue> {
        lift(start_job(
            &mut self.inner,
            JobKind::Export,
            generation,
            cell_size,
            font_size,
            threshold,
            sampling,
            mode,
            ramp,
            &StyleInputs {
                foreground,
                background,
                color_mode,
                contrast,
                invert,
            },
            output_kind,
            frame,
        ))
        .map(JobStage::from)
    }

    /// Returns the chunks completed since the last drain, in order, each exactly
    /// once. An export still sampling has none and returns an empty array.
    ///
    /// # Errors
    ///
    /// Throws `no_job` (also for a preview job), `stale_generation`, `cancelled`,
    /// `disposed`, `resource_limit_exceeded` or `encode_failed`; a terminal error
    /// is stored as the job's failure.
    #[wasm_bindgen(js_name = drainExport, unchecked_return_type = "Uint8Array[]")]
    pub fn drain_export(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "number")] generation: &JsValue,
    ) -> Result<Vec<JsValue>, JsValue> {
        lift(
            generation_of(&self.inner, generation).and_then(|generation| {
                self.inner
                    .drain_export(generation)
                    .map_err(BindingError::from)
            }),
        )
        .map(|chunks| export_chunks(&chunks))
    }

    /// Finalizes the PNG and returns every chunk not yet drained, in order. The
    /// concatenation of all drained and final chunks is the complete PNG file.
    /// The job is released on success, so a second finish reports `no_job`. A
    /// job already `Failed` by `cancel` or `advance` is not released: the call
    /// reports that failure and the job stays until a newer start replaces it.
    ///
    /// # Errors
    ///
    /// Throws `job_incomplete` before the last row is encoded, `no_job` (also for
    /// a preview job or a second finish), `stale_generation`, `cancelled`,
    /// `disposed`, `resource_limit_exceeded` or `encode_failed`.
    #[wasm_bindgen(js_name = finishExport, unchecked_return_type = "Uint8Array[]")]
    pub fn finish_export(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "number")] generation: &JsValue,
    ) -> Result<Vec<JsValue>, JsValue> {
        lift(
            generation_of(&self.inner, generation).and_then(|generation| {
                self.inner
                    .finish_export(generation)
                    .map_err(BindingError::from)
            }),
        )
        .map(|chunks| export_chunks(&chunks))
    }

    /// Releases the source, caches, renderer and job. Idempotent.
    pub fn dispose(&mut self) {
        self.inner.dispose();
    }
}
