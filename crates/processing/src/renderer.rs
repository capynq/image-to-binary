//! Deterministic glyph and block rasterization over an immutable sampled grid.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use fontdue::{Font, FontSettings};
use image_to_binary_core::{
    Cancellation, ContractError, Generation, Progress, RenderMode, Sampling, MAX_CELL_SIZE,
    MAX_CONTRAST, MAX_FONT_SIZE, MAX_SOURCE_CELL_HEIGHT, MIN_CELL_SIZE, MIN_CONTRAST,
    MIN_FONT_SIZE,
};

use crate::style::{matte, mix, toned_luma};
use crate::{ColorMode, RenderStyle, Rgb, SampleCell};

const FONT_BYTES: &[u8] = include_bytes!("../../../assets/fonts/IBMPlexMono-Regular.ttf");
const DEFAULT_CACHE_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_OUTPUT_BYTES: u64 = 256 * 1024 * 1024;
const ALPHA_SCALE: u32 = 255;
const BLOCK_SCALE: u32 = 100;
const GLYPH_SLOTS: usize = 128;
const BLOCK_LEVELS: [(char, u8); 5] = [
    (' ', 0),
    ('\u{2591}', 25),
    ('\u{2592}', 50),
    ('\u{2593}', 75),
    ('\u{2588}', 100),
];

/// Maximum RGBA8 bytes one [`Renderer::render_rows`] call may allocate.
///
/// This ceiling applies independently of [`RenderOptions::max_output_bytes`];
/// the stricter of the two limits is enforced.
pub const MAX_RENDER_ROW_BYTES: u64 = 64 * 1024 * 1024;

/// Returns the checked RGBA8 byte size of a pixel region within both limits.
fn checked_row_budget(
    width: u32,
    height: u32,
    max_output_bytes: u64,
) -> Result<u64, ContractError> {
    let bytes = rgba_bytes(width, height)?;
    if bytes > max_output_bytes || bytes > MAX_RENDER_ROW_BYTES {
        return Err(ContractError::ResourceLimitExceeded);
    }
    Ok(bytes)
}

fn rgba_bytes(width: u32, height: u32) -> Result<u64, ContractError> {
    u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|value| value.checked_mul(4))
        .ok_or(ContractError::ArithmeticOverflow)
}

fn zeroed_pixels(bytes: u64) -> Result<Vec<u8>, ContractError> {
    let count = usize::try_from(bytes).map_err(|_| ContractError::ResourceLimitExceeded)?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(count)
        .map_err(|_| ContractError::ResourceLimitExceeded)?;
    pixels.resize(count, 0);
    Ok(pixels)
}

/// Errors returned by the renderer before a frame is exposed to callers.
#[derive(Debug)]
pub enum RenderError {
    /// A public rendering value is outside its contract.
    Contract(ContractError),
    /// The bundled font could not be parsed.
    InvalidFont(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => error.fmt(formatter),
            Self::InvalidFont(error) => write!(formatter, "bundled font is invalid: {error}"),
        }
    }
}

impl std::error::Error for RenderError {}

impl From<ContractError> for RenderError {
    fn from(error: ContractError) -> Self {
        Self::Contract(error)
    }
}

/// Immutable settings for one rendering pass.
///
/// `mode` other than [`RenderMode::Binary`] requires [`Sampling::Average`];
/// the classic compatibility preset is `Binary` with [`Sampling::Classic`].
/// `style` is applied at render time and never changes a sampled grid. Every
/// public field is validated by the render entry points, which reject values
/// outside the shared bounds in `image_to_binary_core`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderOptions {
    /// Source cell size and output tile size in pixels.
    pub cell_size: u8,
    /// Font size in pixels before preview fitting.
    pub font_size: u8,
    /// Toned luminance below this value selects `1` in binary mode.
    pub threshold: u8,
    /// Sampling geometry used to produce the supplied grid.
    pub sampling: Sampling,
    /// Glyph selection applied to every sampled cell.
    pub mode: RenderMode,
    /// Maximum owned output bytes.
    pub max_output_bytes: u64,
    /// Colour, contrast, and invert controls applied after sampling.
    pub style: RenderStyle,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            cell_size: 20,
            font_size: 20,
            threshold: 128,
            sampling: Sampling::Classic,
            mode: RenderMode::Binary,
            max_output_bytes: DEFAULT_OUTPUT_BYTES,
            style: RenderStyle::default(),
        }
    }
}

impl RenderOptions {
    fn validate(self) -> Result<(), RenderError> {
        if !(MIN_CELL_SIZE..=MAX_CELL_SIZE).contains(&self.cell_size) {
            return Err(ContractError::InvalidSettings("cell_size").into());
        }
        if !(MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&self.font_size) {
            return Err(ContractError::InvalidSettings("font_size").into());
        }
        if !(MIN_CONTRAST..=MAX_CONTRAST).contains(&self.style.contrast) {
            return Err(ContractError::InvalidSettings("contrast").into());
        }
        if self.max_output_bytes == 0 {
            return Err(ContractError::InvalidSettings("output limit").into());
        }
        if self.mode != RenderMode::Binary && self.sampling != Sampling::Average {
            return Err(ContractError::InvalidSettings("render mode sampling").into());
        }
        Ok(())
    }
}

/// Maps one sampled cell to the character selected by `options.mode`.
///
/// Every mode reads the luminance from [`toned_luma`], so contrast and invert
/// apply uniformly. PNG composition calls this for every cell, and any future
/// text export must call it rather than recompute glyphs. Binary delegates its
/// decision to the single threshold comparison in `SampleCell::binary`. Ascii
/// and Block pick a level with `(toned * (levels - 1) + 127) / 255`, which
/// rounds to the nearest level and is non-decreasing in the toned value. Later
/// ramp entries carry more ink, so brighter cells select later entries on the
/// dark default background. Block levels run from space to full block.
#[must_use]
pub fn cell_glyph(cell: SampleCell, options: &RenderOptions) -> char {
    let toned = toned_luma(cell, options);
    match options.mode {
        RenderMode::Binary => char::from(SampleCell::binary(toned, options.threshold)),
        RenderMode::Ascii(ramp) => {
            let index = level_index(usize::from(toned), ramp.len());
            ramp.as_bytes().get(index).copied().map_or(' ', char::from)
        }
        RenderMode::Block => {
            let index = level_index(usize::from(toned), BLOCK_LEVELS.len());
            BLOCK_LEVELS.get(index).map_or(' ', |(glyph, _)| *glyph)
        }
    }
}

fn level_index(luma: usize, levels: usize) -> usize {
    luma.saturating_mul(levels.saturating_sub(1))
        .saturating_add(127)
        / 255
}

fn block_coverage(glyph: char) -> u8 {
    BLOCK_LEVELS
        .iter()
        .find(|(level, _)| *level == glyph)
        .map_or(0, |(_, coverage)| *coverage)
}

/// Owned RGBA8 output with dimensions checked before allocation.
#[derive(Debug, Eq, PartialEq)]
pub struct RenderedFrame {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

/// Result of one bounded preview rendering call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewAdvance {
    InProgress(Progress),
    Completed(Progress),
}

#[derive(Debug)]
struct PreviewRaster {
    width: u32,
    height: u32,
    export_width: u32,
    export_height: u32,
    tile: TileGeometry,
    options: RenderOptions,
    palette: GlyphPalette,
    progress: Progress,
    pixels: Vec<u8>,
}

/// Owned resumable preview rendering task over an immutable sampled grid.
#[derive(Debug)]
pub struct PreviewTask {
    grid: Arc<crate::SampleGrid>,
    raster: PreviewRaster,
    cancellation: Cancellation,
    failed: Option<ContractError>,
}

/// Output cell size in pixels for one rendering pass.
#[derive(Clone, Copy, Debug)]
struct TileGeometry {
    width: u32,
    height: u32,
}

impl RenderedFrame {
    pub(crate) fn from_rgba(
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    ) -> Result<Self, ContractError> {
        let expected = rgba_bytes(width, height)?;
        if width == 0 || height == 0 || u64::try_from(pixels.len()).ok() != Some(expected) {
            return Err(ContractError::InvalidDimensions);
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }
    /// Returns output width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }
    /// Returns output height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }
    /// Borrows the owned RGBA8 pixels.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
    /// Consumes the frame and returns its RGBA8 pixels without copying.
    #[must_use]
    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }
}

/// Bounded glyph-cache accounting for diagnostics and acceptance checks.
///
/// `rasterized` counts every font rasterization, including uncached ones, so
/// callers can prove that a settings change reused existing glyph masks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RendererCacheStats {
    entries: usize,
    bytes: u64,
    capacity: u64,
    rasterized: u64,
}

/// Geometry for the non-classic font-aspect-corrected sampling preset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AspectGeometry {
    /// Horizontal source-cell width in source pixels.
    pub source_cell_width: u8,
    /// Rounded vertical source-cell height in source pixels.
    pub source_cell_height: u16,
    /// Output glyph-cell width in pixels.
    pub output_cell_width: u16,
    /// Output glyph-cell height in pixels.
    pub output_cell_height: u16,
}

impl RendererCacheStats {
    #[must_use]
    pub const fn entries(self) -> usize {
        self.entries
    }
    #[must_use]
    pub const fn bytes(self) -> u64 {
        self.bytes
    }
    #[must_use]
    pub const fn capacity(self) -> u64 {
        self.capacity
    }
    /// Returns the number of font rasterizations performed by this renderer.
    #[must_use]
    pub const fn rasterized(self) -> u64 {
        self.rasterized
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct GlyphKey {
    character: char,
    size_bits: u32,
}

#[derive(Clone, Debug)]
struct GlyphMask {
    width: usize,
    height: usize,
    xmin: i32,
    ymin: i32,
    bitmap: Vec<u8>,
    bytes: u64,
}

#[derive(Debug)]
struct GlyphAtlas {
    entries: HashMap<GlyphKey, Arc<GlyphMask>>,
    order: VecDeque<GlyphKey>,
    bytes: u64,
    capacity: u64,
    rasterized: u64,
}

impl GlyphAtlas {
    fn new(capacity: u64) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
            capacity,
            rasterized: 0,
        }
    }

    fn get(
        &mut self,
        font: &Font,
        character: char,
        size: f32,
    ) -> Result<Arc<GlyphMask>, RenderError> {
        let key = GlyphKey {
            character,
            size_bits: size.to_bits(),
        };
        if self.entries.contains_key(&key) {
            self.order.retain(|item| *item != key);
            self.order.push_back(key);
            return self
                .entries
                .get(&key)
                .cloned()
                .ok_or(ContractError::ArithmeticOverflow.into());
        }
        self.rasterized = self
            .rasterized
            .checked_add(1)
            .ok_or(ContractError::ArithmeticOverflow)?;
        let (metrics, bitmap) = font.rasterize(character, size);
        let bytes =
            u64::try_from(bitmap.capacity()).map_err(|_| ContractError::ArithmeticOverflow)?;
        if self.capacity != 0 && bytes <= self.capacity {
            while self.bytes > self.capacity - bytes {
                if let Some(old) = self.order.pop_front() {
                    if let Some(entry) = self.entries.remove(&old) {
                        self.bytes = self
                            .bytes
                            .checked_sub(entry.bytes)
                            .ok_or(ContractError::ArithmeticOverflow)?;
                    }
                } else {
                    break;
                }
            }
            self.bytes = self
                .bytes
                .checked_add(bytes)
                .ok_or(ContractError::ArithmeticOverflow)?;
            self.order.push_back(key);
            let mask = Arc::new(GlyphMask {
                width: metrics.width,
                height: metrics.height,
                xmin: metrics.xmin,
                ymin: metrics.ymin,
                bitmap,
                bytes,
            });
            self.entries.insert(key, Arc::clone(&mask));
            return Ok(mask);
        }
        Ok(Arc::new(GlyphMask {
            width: metrics.width,
            height: metrics.height,
            xmin: metrics.xmin,
            ymin: metrics.ymin,
            bitmap,
            bytes,
        }))
    }
}

/// Glyph masks needed by one rendering pass, indexed by ASCII code.
///
/// Holding `Arc` handles keeps a pass's masks alive independently of cache
/// eviction; the bounded atlas still governs what is retained between passes.
#[derive(Debug)]
struct GlyphPalette {
    masks: Vec<Option<Arc<GlyphMask>>>,
}

impl GlyphPalette {
    fn get(&self, glyph: char) -> Result<&GlyphMask, ContractError> {
        self.masks
            .get(palette_slot(glyph)?)
            .and_then(Option::as_ref)
            .map(Arc::as_ref)
            .ok_or(ContractError::InvalidSettings("glyph palette"))
    }
}

fn palette_slot(glyph: char) -> Result<usize, ContractError> {
    usize::try_from(u32::from(glyph))
        .ok()
        .filter(|slot| *slot < GLYPH_SLOTS)
        .ok_or(ContractError::InvalidSettings("glyph palette"))
}

/// Owns the parsed bundled font and private bounded glyph masks.
#[derive(Debug)]
pub struct Renderer {
    font: Font,
    atlas: GlyphAtlas,
}

impl Renderer {
    fn validate_grid(
        &self,
        grid: &crate::SampleGrid,
        options: RenderOptions,
    ) -> Result<(), RenderError> {
        if grid.sampling() != options.sampling || grid.source_cell_width() != options.cell_size {
            return Err(ContractError::InvalidSettings("sampling geometry").into());
        }
        match options.sampling {
            Sampling::Classic if grid.source_cell_height() != u32::from(options.cell_size) => {
                Err(ContractError::InvalidSettings("sampling geometry").into())
            }
            Sampling::Average => {
                let geometry = self.aspect_geometry(options.cell_size, options.font_size)?;
                if grid.source_cell_height() != u32::from(geometry.source_cell_height) {
                    return Err(ContractError::InvalidSettings("sampling geometry").into());
                }
                Ok(())
            }
            Sampling::Classic => Ok(()),
        }
    }

    /// Creates a renderer using the bundled IBM Plex Mono font.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::InvalidFont`] if the embedded font is malformed.
    pub fn new() -> Result<Self, RenderError> {
        let font = Font::from_bytes(FONT_BYTES, FontSettings::default())
            .map_err(|error| RenderError::InvalidFont(error.to_string()))?;
        Ok(Self {
            font,
            atlas: GlyphAtlas::new(DEFAULT_CACHE_BYTES),
        })
    }

    /// Creates a renderer with an explicit glyph-mask byte ceiling.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::InvalidFont`] if the embedded font is malformed.
    pub fn with_cache_capacity(capacity: u64) -> Result<Self, RenderError> {
        let mut renderer = Self::new()?;
        renderer.atlas = GlyphAtlas::new(capacity);
        Ok(renderer)
    }

    /// Returns private cache accounting without exposing cache entries.
    #[must_use]
    pub fn cache_stats(&self) -> RendererCacheStats {
        RendererCacheStats {
            entries: self.atlas.entries.len(),
            bytes: self.atlas.bytes,
            capacity: self.atlas.capacity,
            rasterized: self.atlas.rasterized,
        }
    }

    /// Measures the bundled monospace advance and line height for new presets.
    ///
    /// Classic rendering does not use this geometry. The returned source-cell
    /// height is rounded and bounded for rectangular average sampling; callers
    /// should expect at most one source-cell of aspect-ratio edge rounding.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Contract`] when either input is outside its range.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub fn aspect_geometry(
        &self,
        cell_width: u8,
        font_size: u8,
    ) -> Result<AspectGeometry, RenderError> {
        if !(MIN_CELL_SIZE..=MAX_CELL_SIZE).contains(&cell_width) {
            return Err(ContractError::InvalidSettings("cell_size").into());
        }
        if !(MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&font_size) {
            return Err(ContractError::InvalidSettings("font_size").into());
        }
        let size = f32::from(font_size);
        let (metrics, _) = self.font.rasterize('0', size);
        let line_height = self
            .font
            .horizontal_line_metrics(size)
            .map_or(metrics.height as f32, |value| value.new_line_size);
        let advance = metrics.advance_width.max(1.0);
        let source_height = (f32::from(cell_width) * line_height / advance)
            .round()
            .clamp(1.0, f32::from(MAX_SOURCE_CELL_HEIGHT)) as u16;
        let output_height = line_height.round().clamp(1.0, f32::from(u16::MAX)) as u16;
        Ok(AspectGeometry {
            source_cell_width: cell_width,
            source_cell_height: source_height,
            output_cell_width: metrics.advance_width.round().max(1.0) as u16,
            output_cell_height: output_height,
        })
    }

    /// Renders a complete RGBA8 frame from the caller-owned source grid.
    ///
    /// # Errors
    ///
    /// Returns a contract error when settings, dimensions, or output allocation
    /// exceed their configured limits.
    pub fn render(
        &mut self,
        grid: &crate::SampleGrid,
        options: RenderOptions,
    ) -> Result<RenderedFrame, RenderError> {
        options.validate()?;
        self.validate_grid(grid, options)?;
        let tile = self.tile_geometry(options)?;
        let width = grid
            .width()
            .checked_mul(tile.width)
            .ok_or(ContractError::ArithmeticOverflow)?;
        let height = grid
            .height()
            .checked_mul(tile.height)
            .ok_or(ContractError::ArithmeticOverflow)?;
        let bytes = rgba_bytes(width, height)?;
        if bytes > options.max_output_bytes {
            return Err(ContractError::ResourceLimitExceeded.into());
        }
        let mut pixels = zeroed_pixels(bytes)?;
        let palette = self.glyph_palette(grid, 0..grid.height(), options)?;
        let composition = Composition {
            grid,
            options,
            palette: &palette,
            tile,
        };
        let frame_width = usize::try_from(width).map_err(|_| ContractError::ArithmeticOverflow)?;
        for row in 0..grid.height() {
            composition.paint_row(row, row, frame_width, &mut pixels)?;
        }
        Ok(RenderedFrame {
            width,
            height,
            pixels,
        })
    }

    /// Renders a bounded preview while preserving the source grid geometry.
    ///
    /// # Errors
    ///
    /// Returns a contract error when settings, dimensions, or output allocation
    /// exceed their configured limits.
    pub fn render_preview(
        &mut self,
        grid: &crate::SampleGrid,
        options: RenderOptions,
    ) -> Result<RenderedFrame, RenderError> {
        options.validate()?;
        self.validate_grid(grid, options)?;
        let raster = self.preview_raster(grid, options)?;
        let mut raster = raster;
        raster.advance(grid, u64::MAX)?;
        Ok(raster.finish())
    }

    /// Creates an owned resumable preview task for one generation.
    ///
    /// Grid validation, output reservation, and glyph rasterization happen
    /// synchronously. Those setup operations cannot be preempted; each later
    /// advance processes at most its requested number of output pixels.
    ///
    /// # Errors
    ///
    /// Returns a contract error when settings, dimensions, or output allocation
    /// exceed their configured limits.
    pub fn preview_task(
        &mut self,
        grid: Arc<crate::SampleGrid>,
        options: RenderOptions,
        generation: Generation,
    ) -> Result<PreviewTask, RenderError> {
        options.validate()?;
        self.validate_grid(&grid, options)?;
        let raster = self.preview_raster(&grid, options)?;
        Ok(PreviewTask {
            grid,
            raster,
            cancellation: Cancellation::new(generation),
            failed: None,
        })
    }

    fn preview_raster(
        &mut self,
        grid: &crate::SampleGrid,
        options: RenderOptions,
    ) -> Result<PreviewRaster, RenderError> {
        let tile = self.tile_geometry(options)?;
        let export_width = u64::from(grid.width())
            .checked_mul(u64::from(tile.width))
            .ok_or(ContractError::ArithmeticOverflow)?;
        let export_height = u64::from(grid.height())
            .checked_mul(u64::from(tile.height))
            .ok_or(ContractError::ArithmeticOverflow)?;
        let longest = export_width.max(export_height);
        let target_width = if longest > 1600 {
            export_width.saturating_mul(1600) / longest
        } else {
            export_width
        };
        let target_height = if longest > 1600 {
            export_height.saturating_mul(1600) / longest
        } else {
            export_height
        };
        let width =
            u32::try_from(target_width.max(1)).map_err(|_| ContractError::ResourceLimitExceeded)?;
        let height = u32::try_from(target_height.max(1))
            .map_err(|_| ContractError::ResourceLimitExceeded)?;
        let export_width =
            u32::try_from(export_width).map_err(|_| ContractError::ResourceLimitExceeded)?;
        let export_height =
            u32::try_from(export_height).map_err(|_| ContractError::ResourceLimitExceeded)?;
        let bytes = rgba_bytes(width, height)?;
        if bytes > options.max_output_bytes {
            return Err(ContractError::ResourceLimitExceeded.into());
        }
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(
                usize::try_from(bytes).map_err(|_| ContractError::ResourceLimitExceeded)?,
            )
            .map_err(|_| ContractError::ResourceLimitExceeded)?;
        let palette = self.glyph_palette(grid, 0..grid.height(), options)?;
        let total = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(ContractError::ArithmeticOverflow)?;
        Ok(PreviewRaster {
            width,
            height,
            export_width,
            export_height,
            tile,
            options,
            palette,
            progress: Progress::new(total)?,
            pixels,
        })
    }

    /// Renders a contiguous row batch for streaming encoders.
    ///
    /// `start` and `count` address source-grid rows. The returned buffer
    /// contains `count * output_cell_height` pixel rows laid out as RGBA8
    /// pixels with the full frame width. Concatenating batches that cover every
    /// row yields exactly the bytes of [`Renderer::render`].
    ///
    /// The buffer may not exceed [`MAX_RENDER_ROW_BYTES`] or
    /// `options.max_output_bytes`; both are checked before allocation.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidDimensions`] for a zero `count` or a
    /// range outside the grid, [`ContractError::ResourceLimitExceeded`] when the
    /// buffer exceeds either limit, or another contract error for invalid
    /// settings, grid geometry, or arithmetic overflow.
    pub fn render_rows(
        &mut self,
        grid: &crate::SampleGrid,
        options: RenderOptions,
        start: u32,
        count: u32,
    ) -> Result<Vec<u8>, RenderError> {
        options.validate()?;
        self.validate_grid(grid, options)?;
        if count == 0 {
            return Err(ContractError::InvalidDimensions.into());
        }
        let end = start
            .checked_add(count)
            .ok_or(ContractError::ArithmeticOverflow)?;
        if end > grid.height() {
            return Err(ContractError::InvalidDimensions.into());
        }
        let tile = self.tile_geometry(options)?;
        let width = grid
            .width()
            .checked_mul(tile.width)
            .ok_or(ContractError::ArithmeticOverflow)?;
        let height = count
            .checked_mul(tile.height)
            .ok_or(ContractError::ArithmeticOverflow)?;
        let bytes = checked_row_budget(width, height, options.max_output_bytes)?;
        let mut pixels = zeroed_pixels(bytes)?;
        let palette = self.glyph_palette(grid, start..end, options)?;
        let composition = Composition {
            grid,
            options,
            palette: &palette,
            tile,
        };
        let frame_width = usize::try_from(width).map_err(|_| ContractError::ArithmeticOverflow)?;
        for source_row in start..end {
            composition.paint_row(source_row, source_row - start, frame_width, &mut pixels)?;
        }
        Ok(pixels)
    }

    /// Returns checked export dimensions without allocating a frame.
    ///
    /// # Errors
    ///
    /// Returns a contract error when grid geometry does not match options or
    /// the derived dimensions overflow.
    pub fn frame_dimensions(
        &self,
        grid: &crate::SampleGrid,
        options: RenderOptions,
    ) -> Result<(u32, u32), RenderError> {
        options.validate()?;
        self.validate_grid(grid, options)?;
        let tile = self.tile_geometry(options)?;
        Ok((
            grid.width()
                .checked_mul(tile.width)
                .ok_or(ContractError::ArithmeticOverflow)?,
            grid.height()
                .checked_mul(tile.height)
                .ok_or(ContractError::ArithmeticOverflow)?,
        ))
    }

    fn tile_geometry(&self, options: RenderOptions) -> Result<TileGeometry, RenderError> {
        if options.sampling == Sampling::Average {
            let geometry = self.aspect_geometry(options.cell_size, options.font_size)?;
            Ok(TileGeometry {
                width: u32::from(geometry.output_cell_width),
                height: u32::from(geometry.output_cell_height),
            })
        } else {
            let cell = u32::from(options.cell_size);
            Ok(TileGeometry {
                width: cell,
                height: cell,
            })
        }
    }

    /// Fetches only the glyph masks that `rows` select, so cache traffic matches
    /// the characters a pass actually draws. Block mode draws no font glyphs.
    fn glyph_palette(
        &mut self,
        grid: &crate::SampleGrid,
        rows: Range<u32>,
        options: RenderOptions,
    ) -> Result<GlyphPalette, RenderError> {
        let mut required = [false; GLYPH_SLOTS];
        if options.mode != RenderMode::Block {
            for row in rows {
                for column in 0..grid.width() {
                    let cell = grid
                        .cell(column, row)
                        .ok_or(ContractError::InvalidDimensions)?;
                    required[palette_slot(cell_glyph(cell, &options))?] = true;
                }
            }
        }
        let size = f32::from(options.font_size);
        let mut masks = vec![None; GLYPH_SLOTS];
        for (slot, needed) in required.iter().enumerate() {
            if *needed {
                let glyph = char::from(u8::try_from(slot).map_err(|_| ContractError::InvalidRamp)?);
                masks[slot] = Some(self.atlas.get(&self.font, glyph, size)?);
            }
        }
        Ok(GlyphPalette { masks })
    }
}

impl PreviewRaster {
    fn advance(
        &mut self,
        grid: &crate::SampleGrid,
        pixel_budget: u64,
    ) -> Result<(), ContractError> {
        let remaining = self.progress.total() - self.progress.completed();
        let count = pixel_budget.min(remaining);
        let width = u64::from(self.width);
        let composition = Composition {
            grid,
            options: self.options,
            palette: &self.palette,
            tile: self.tile,
        };
        for offset in 0..count {
            let position = self.progress.completed() + offset;
            let x =
                u32::try_from(position % width).map_err(|_| ContractError::ArithmeticOverflow)?;
            let y =
                u32::try_from(position / width).map_err(|_| ContractError::ArithmeticOverflow)?;
            let source_x = u32::try_from(
                (u64::from(x) * 2 + 1) * u64::from(self.export_width) / (2 * u64::from(self.width)),
            )
            .map_err(|_| ContractError::ArithmeticOverflow)?
            .min(self.export_width - 1);
            let source_y = u32::try_from(
                (u64::from(y) * 2 + 1) * u64::from(self.export_height)
                    / (2 * u64::from(self.height)),
            )
            .map_err(|_| ContractError::ArithmeticOverflow)?
            .min(self.export_height - 1);
            let pixel = composition.export_pixel(source_x, source_y)?;
            self.pixels.extend_from_slice(&pixel);
        }
        self.progress.advance(count)?;
        Ok(())
    }

    fn finish(self) -> RenderedFrame {
        RenderedFrame {
            width: self.width,
            height: self.height,
            pixels: self.pixels,
        }
    }
}

impl PreviewTask {
    /// Returns output pixels completed and the total preview pixel count.
    #[must_use]
    pub const fn progress(&self) -> Progress {
        self.raster.progress
    }

    /// Requests cancellation; the next call boundary discards partial pixels.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    /// Renders at most `pixel_budget` output pixels in row-major order.
    ///
    /// A zero budget is rejected without changing task state. Cancellation,
    /// stale generations, and internal rendering failures permanently
    /// invalidate the task and release its raster buffer.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidProgress`] for a zero budget, or a
    /// cancellation, generation, arithmetic, or grid contract error.
    pub fn advance(
        &mut self,
        pixel_budget: u64,
        current_generation: Generation,
    ) -> Result<PreviewAdvance, ContractError> {
        if pixel_budget == 0 {
            return Err(ContractError::InvalidProgress);
        }
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        if let Err(error) = self.cancellation.check(current_generation) {
            self.failed = Some(error.clone());
            self.raster.pixels = Vec::new();
            return Err(error);
        }
        let result = self.raster.advance(&self.grid, pixel_budget);
        if let Err(error) = self.cancellation.check(current_generation) {
            self.failed = Some(error.clone());
            self.raster.pixels = Vec::new();
            return Err(error);
        }
        if let Err(error) = result {
            self.failed = Some(error.clone());
            self.raster.pixels = Vec::new();
            return Err(error);
        }
        Ok(
            if self.raster.progress.completed() == self.raster.progress.total() {
                PreviewAdvance::Completed(self.raster.progress)
            } else {
                PreviewAdvance::InProgress(self.raster.progress)
            },
        )
    }

    /// Consumes a complete, current-generation task into its rendered frame.
    ///
    /// # Errors
    ///
    /// Returns cancellation or stale-generation errors, or
    /// [`ContractError::InvalidProgress`] when rendering is incomplete. Returns
    /// a previously recorded terminal rendering error unchanged.
    pub fn finish(
        mut self,
        current_generation: Generation,
    ) -> Result<RenderedFrame, ContractError> {
        if let Some(error) = self.failed.take() {
            return Err(error);
        }
        if let Err(error) = self.cancellation.check(current_generation) {
            self.raster.pixels = Vec::new();
            return Err(error);
        }
        if self.raster.progress.completed() != self.raster.progress.total() {
            return Err(ContractError::InvalidProgress);
        }
        Ok(self.raster.finish())
    }
}

/// Borrowed inputs shared by every output path; each pixel is computed here alone.
#[derive(Clone, Copy)]
struct Composition<'a> {
    grid: &'a crate::SampleGrid,
    options: RenderOptions,
    palette: &'a GlyphPalette,
    tile: TileGeometry,
}

impl<'a> Composition<'a> {
    fn paint(self, cell: SampleCell) -> Result<CellPaint<'a>, ContractError> {
        let glyph = cell_glyph(cell, &self.options);
        let foreground = self.foreground(cell)?;
        match self.options.mode {
            RenderMode::Block => Ok(CellPaint::Block {
                coverage: block_coverage(glyph),
                foreground,
            }),
            RenderMode::Binary | RenderMode::Ascii(_) => Ok(CellPaint::Glyph {
                mask: self.palette.get(glyph)?,
                foreground,
            }),
        }
    }

    fn foreground(self, cell: SampleCell) -> Result<Rgb, ContractError> {
        match self.options.style.color {
            ColorMode::Monochrome => Ok(self.options.style.foreground),
            ColorMode::Source => matte(cell.color(), self.options.style.background),
        }
    }

    fn export_pixel(self, x: u32, y: u32) -> Result<[u8; 4], ContractError> {
        let column = x / self.tile.width;
        let row = y / self.tile.height;
        let cell = self
            .grid
            .cell(column, row)
            .ok_or(ContractError::InvalidDimensions)?;
        self.paint(cell)?.pixel(
            self.options.style.background,
            self.tile,
            x % self.tile.width,
            y % self.tile.height,
        )
    }

    fn paint_row(
        self,
        source_row: u32,
        output_row: u32,
        frame_width: usize,
        pixels: &mut [u8],
    ) -> Result<(), ContractError> {
        let top = output_row
            .checked_mul(self.tile.height)
            .ok_or(ContractError::ArithmeticOverflow)?;
        for column in 0..self.grid.width() {
            let cell = self
                .grid
                .cell(column, source_row)
                .ok_or(ContractError::InvalidDimensions)?;
            let paint = self.paint(cell)?;
            let background = self.options.style.background;
            let left = column
                .checked_mul(self.tile.width)
                .ok_or(ContractError::ArithmeticOverflow)?;
            for local_y in 0..self.tile.height {
                let y = top
                    .checked_add(local_y)
                    .ok_or(ContractError::ArithmeticOverflow)?;
                for local_x in 0..self.tile.width {
                    let x = left
                        .checked_add(local_x)
                        .ok_or(ContractError::ArithmeticOverflow)?;
                    let pixel = paint.pixel(background, self.tile, local_x, local_y)?;
                    let index = usize::try_from(y)
                        .ok()
                        .and_then(|row| row.checked_mul(frame_width))
                        .and_then(|offset| offset.checked_add(usize::try_from(x).ok()?))
                        .and_then(|pixel| pixel.checked_mul(4))
                        .ok_or(ContractError::ArithmeticOverflow)?;
                    let end = index
                        .checked_add(4)
                        .ok_or(ContractError::ArithmeticOverflow)?;
                    pixels
                        .get_mut(index..end)
                        .ok_or(ContractError::InvalidDimensions)?
                        .copy_from_slice(&pixel);
                }
            }
        }
        Ok(())
    }
}

/// Resolved paint for one sampled cell: a font mask or a block coverage percent,
/// plus the cell's opaque foreground colour.
#[derive(Clone, Copy)]
enum CellPaint<'a> {
    Glyph {
        mask: &'a GlyphMask,
        foreground: Rgb,
    },
    Block {
        coverage: u8,
        foreground: Rgb,
    },
}

impl CellPaint<'_> {
    fn pixel(
        self,
        background: Rgb,
        tile: TileGeometry,
        local_x: u32,
        local_y: u32,
    ) -> Result<[u8; 4], ContractError> {
        match self {
            Self::Glyph { mask, foreground } => blend(
                foreground,
                background,
                u32::from(glyph_alpha(mask, tile, local_x, local_y)?),
                ALPHA_SCALE,
            ),
            Self::Block {
                coverage,
                foreground,
            } => blend(foreground, background, u32::from(coverage), BLOCK_SCALE),
        }
    }
}

/// Mixes an opaque foreground over an opaque background at `coverage / scale`.
///
/// The result has alpha 255 and rounds half up, so zero coverage yields the exact
/// background and full coverage yields the exact foreground.
fn blend(
    foreground: Rgb,
    background: Rgb,
    coverage: u32,
    scale: u32,
) -> Result<[u8; 4], ContractError> {
    let [red, green, blue] = mix(foreground, background, coverage, scale)?;
    Ok([red, green, blue, u8::MAX])
}

fn glyph_alpha(mask: &GlyphMask, tile: TileGeometry, x: u32, y: u32) -> Result<u8, ContractError> {
    let mask_width = i64::try_from(mask.width).map_err(|_| ContractError::ArithmeticOverflow)?;
    let mask_height = i64::try_from(mask.height).map_err(|_| ContractError::ArithmeticOverflow)?;
    let glyph_left = (i64::from(tile.width) - mask_width) / 2 + i64::from(mask.xmin);
    let baseline = i64::midpoint(i64::from(tile.height), mask_height) + i64::from(mask.ymin);
    let glyph_x = i64::from(x) - glyph_left;
    let glyph_y = i64::from(y) - (baseline - i64::from(mask.ymin) - mask_height);
    if glyph_x < 0 || glyph_y < 0 || glyph_x >= mask_width || glyph_y >= mask_height {
        return Ok(0);
    }
    let row = usize::try_from(glyph_y).map_err(|_| ContractError::ArithmeticOverflow)?;
    let column = usize::try_from(glyph_x).map_err(|_| ContractError::ArithmeticOverflow)?;
    let index = row
        .checked_mul(mask.width)
        .and_then(|offset| offset.checked_add(column))
        .ok_or(ContractError::ArithmeticOverflow)?;
    mask.bitmap
        .get(index)
        .copied()
        .ok_or(ContractError::InvalidDimensions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode, DecodeLimits, Sampler};
    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
    use image_to_binary_core::{
        Ramp, MAX_CELL_SIZE, MAX_CONTRAST, MAX_FONT_SIZE, MAX_RAMP_LENGTH, MAX_THRESHOLD,
        MIN_CELL_SIZE, MIN_CONTRAST, MIN_FONT_SIZE, MIN_THRESHOLD,
    };
    use std::io::Cursor;
    use std::sync::Arc;

    fn grid_with_height(
        width: u32,
        height: u32,
        dark: bool,
        sampling: Sampling,
        cell_height: u8,
    ) -> Arc<crate::SampleGrid> {
        let image = ImageBuffer::from_fn(width, height, |_x, _y| {
            if dark {
                Rgba([0, 0, 0, 255])
            } else {
                Rgba([255, 255, 255, 255])
            }
        });
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        let source = Arc::new(decode(&bytes.into_inner(), DecodeLimits::default()).unwrap());
        let mut sampler = Sampler::new(source);
        if sampling == Sampling::Average {
            sampler
                .sample_average_rect(5, u32::from(cell_height))
                .unwrap()
        } else {
            sampler.sample(5, sampling).unwrap()
        }
    }

    fn grid(width: u32, height: u32, dark: bool, sampling: Sampling) -> Arc<crate::SampleGrid> {
        grid_with_height(width, height, dark, sampling, 5)
    }

    #[test]
    fn row_budget_enforces_fixed_ceiling_independent_of_configured_limit() {
        let pixels = u32::try_from(MAX_RENDER_ROW_BYTES / 4).unwrap();
        assert_eq!(
            checked_row_budget(pixels, 1, u64::MAX),
            Ok(MAX_RENDER_ROW_BYTES)
        );
        assert_eq!(
            checked_row_budget(pixels + 1, 1, u64::MAX),
            Err(ContractError::ResourceLimitExceeded)
        );
    }

    #[test]
    fn row_budget_enforces_configured_limit_and_overflow() {
        assert_eq!(checked_row_budget(2, 3, 24), Ok(24));
        assert_eq!(
            checked_row_budget(2, 3, 23),
            Err(ContractError::ResourceLimitExceeded)
        );
        assert_eq!(checked_row_budget(0, 3, 1), Ok(0));
        assert_eq!(
            checked_row_budget(u32::MAX, u32::MAX, u64::MAX),
            Err(ContractError::ArithmeticOverflow)
        );
    }

    fn patterned_grid(
        width: u32,
        height: u32,
        sampling: Sampling,
        cell_height: u8,
    ) -> Arc<crate::SampleGrid> {
        let image = ImageBuffer::from_fn(width, height, |x, y| {
            if (x / 5 + y / 5) % 2 == 0 {
                Rgba([0, 0, 0, 255])
            } else {
                Rgba([255, 255, 255, 255])
            }
        });
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        let source = Arc::new(decode(&bytes.into_inner(), DecodeLimits::default()).unwrap());
        let mut sampler = Sampler::new(source);
        if sampling == Sampling::Average {
            sampler
                .sample_average_rect(5, u32::from(cell_height))
                .unwrap()
        } else {
            sampler.sample(5, sampling).unwrap()
        }
    }

    fn inverse_preview_oracle(
        renderer: &mut Renderer,
        grid: &crate::SampleGrid,
        options: RenderOptions,
    ) -> RenderedFrame {
        let full = renderer.render(grid, options).unwrap();
        let export_width = full.width();
        let export_height = full.height();
        let longest = export_width.max(export_height);
        let width = if longest > 1600 {
            u32::try_from(u64::from(export_width) * 1600 / u64::from(longest)).unwrap()
        } else {
            export_width
        };
        let height = if longest > 1600 {
            u32::try_from(u64::from(export_height) * 1600 / u64::from(longest)).unwrap()
        } else {
            export_height
        };
        let mut pixels = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let source_x = u32::try_from(
                    ((u64::from(x) * 2 + 1) * u64::from(export_width)) / (2 * u64::from(width)),
                )
                .unwrap();
                let source_y = u32::try_from(
                    ((u64::from(y) * 2 + 1) * u64::from(export_height)) / (2 * u64::from(height)),
                )
                .unwrap();
                let source_x = source_x.min(export_width - 1);
                let source_y = source_y.min(export_height - 1);
                let index = (usize::try_from(source_y).unwrap()
                    * usize::try_from(export_width).unwrap()
                    + usize::try_from(source_x).unwrap())
                    * 4;
                pixels.extend_from_slice(&full.pixels()[index..index + 4]);
            }
        }
        RenderedFrame {
            width,
            height,
            pixels,
        }
    }

    #[test]
    fn repeated_classic_render_is_byte_stable_and_opaque() {
        let grid = grid(10, 10, true, Sampling::Classic);
        let options = RenderOptions {
            cell_size: 5,
            font_size: 8,
            ..RenderOptions::default()
        };
        let mut renderer = Renderer::new().unwrap();
        let first = renderer.render(&grid, options).unwrap();
        let second = renderer.render(&grid, options).unwrap();
        assert_eq!(
            &first.pixels()[..16],
            &[0, 64, 0, 255, 0, 140, 0, 255, 0, 173, 0, 255, 0, 0, 0, 255]
        );
        assert_eq!(first, second);
        assert!(first.pixels().chunks_exact(4).all(|pixel| pixel[3] == 255));
        assert!(first.pixels().chunks_exact(4).any(|pixel| pixel[1] > 0));
    }

    #[test]
    fn preview_longest_edge_is_bounded_for_narrow_grid() {
        let grid = grid(8_050, 5, false, Sampling::Classic);
        let options = RenderOptions {
            cell_size: 5,
            font_size: 8,
            ..RenderOptions::default()
        };
        let mut renderer = Renderer::new().unwrap();
        let preview = renderer.render_preview(&grid, options).unwrap();
        assert!(preview.width().max(preview.height()) <= 1600);
        let mut task = renderer
            .preview_task(Arc::clone(&grid), options, Generation::new(3))
            .unwrap();
        assert_eq!(
            task.progress().total(),
            u64::from(preview.width()) * u64::from(preview.height())
        );
        while task.progress().completed() < task.progress().total() {
            task.advance(4096, Generation::new(3)).unwrap();
        }
        assert_eq!(task.finish(Generation::new(3)).unwrap(), preview);
    }

    #[test]
    fn incremental_preview_matches_inverse_oracle_for_narrow_downscaled_grid() {
        let grid = patterned_grid(10_005, 25, Sampling::Classic, 5);
        let options = RenderOptions {
            cell_size: 5,
            font_size: 8,
            ..RenderOptions::default()
        };
        let expected = inverse_preview_oracle(&mut Renderer::new().unwrap(), &grid, options);
        assert_eq!((expected.width(), expected.height()), (1600, 3));
        for budget in [1, 17, 4096, u64::MAX] {
            let mut task = Renderer::new()
                .unwrap()
                .preview_task(Arc::clone(&grid), options, Generation::new(9))
                .unwrap();
            while task.progress().completed() < task.progress().total() {
                task.advance(budget, Generation::new(9)).unwrap();
            }
            assert_eq!(task.finish(Generation::new(9)).unwrap(), expected);
        }
    }

    #[test]
    fn rows_concatenate_to_full_frame() {
        let grid = grid(20, 20, true, Sampling::Classic);
        let options = RenderOptions {
            cell_size: 5,
            font_size: 30,
            ..RenderOptions::default()
        };
        let mut renderer = Renderer::new().unwrap();
        let full = renderer.render(&grid, options).unwrap();
        let mut rows = Vec::new();
        rows.extend(renderer.render_rows(&grid, options, 0, 2).unwrap());
        rows.extend(renderer.render_rows(&grid, options, 2, 2).unwrap());
        assert_eq!(full.pixels(), rows.as_slice());
    }

    #[test]
    fn cache_capacity_is_respected_and_options_are_checked() {
        let grid = grid(10, 10, true, Sampling::Classic);
        let options = RenderOptions {
            cell_size: 5,
            font_size: 8,
            ..RenderOptions::default()
        };
        let mut renderer = Renderer::with_cache_capacity(1).unwrap();
        renderer.render(&grid, options).unwrap();
        assert!(renderer.cache_stats().bytes() <= renderer.cache_stats().capacity());
        assert!(renderer
            .render(
                &grid,
                RenderOptions {
                    font_size: 7,
                    ..options
                }
            )
            .is_err());
        assert!(renderer
            .render(
                &grid,
                RenderOptions {
                    max_output_bytes: 1,
                    ..options
                }
            )
            .is_err());
    }

    #[test]
    fn average_geometry_is_rectangular_and_measured() {
        let renderer = Renderer::new().unwrap();
        let geometry = renderer.aspect_geometry(5, 20).unwrap();
        assert!(geometry.source_cell_height > 0);
        assert!(geometry.output_cell_height > geometry.output_cell_width);
        let grid = grid_with_height(
            20,
            20,
            true,
            Sampling::Average,
            u8::try_from(geometry.source_cell_height).unwrap(),
        );
        let mut renderer = Renderer::new().unwrap();
        let frame = renderer
            .render(
                &grid,
                RenderOptions {
                    cell_size: 5,
                    sampling: Sampling::Average,
                    ..RenderOptions::default()
                },
            )
            .unwrap();
        assert_eq!(
            frame.width(),
            grid.width() * u32::from(geometry.output_cell_width)
        );
    }

    #[test]
    fn incremental_preview_matches_inverse_oracle_for_both_sampling_modes() {
        let mut cases = vec![(Sampling::Classic, 8, 5)];
        let geometry_renderer = Renderer::new().unwrap();
        let average_height = u8::try_from(
            geometry_renderer
                .aspect_geometry(5, 30)
                .unwrap()
                .source_cell_height,
        )
        .unwrap();
        cases.push((Sampling::Average, 30, average_height));
        for (sampling, font_size, cell_height) in cases {
            let grid = patterned_grid(85, 61, sampling, cell_height);
            let options = RenderOptions {
                cell_size: 5,
                font_size,
                sampling,
                ..RenderOptions::default()
            };
            let expected = inverse_preview_oracle(&mut Renderer::new().unwrap(), &grid, options);
            assert!(expected.pixels().chunks_exact(4).any(|pixel| pixel[1] > 0));
            assert!(expected.pixels().chunks_exact(4).any(|pixel| pixel[1] == 0));
            for budget in [1, 17, 4096, u64::MAX] {
                let mut renderer = Renderer::with_cache_capacity(1).unwrap();
                let task = renderer
                    .preview_task(Arc::clone(&grid), options, Generation::new(42))
                    .unwrap();
                assert!(renderer.cache_stats().bytes() <= renderer.cache_stats().capacity());
                drop(renderer);
                let mut task = task;
                while task.progress().completed() < task.progress().total() {
                    let before = task.progress().completed();
                    let advance = task.advance(budget, Generation::new(42)).unwrap();
                    let completed = task.progress().completed();
                    assert!(completed - before <= budget);
                    assert!(completed > before);
                    if completed == task.progress().total() {
                        assert_eq!(advance, PreviewAdvance::Completed(task.progress()));
                    }
                }
                assert_eq!(task.finish(Generation::new(42)).unwrap(), expected);
            }
            assert_eq!(
                Renderer::new()
                    .unwrap()
                    .render_preview(&grid, options)
                    .unwrap(),
                expected
            );
        }
    }

    #[test]
    fn preview_task_zero_budget_cancel_stale_and_finish_contracts() {
        let patterned = patterned_grid(50, 50, Sampling::Classic, 5);
        let options = RenderOptions {
            cell_size: 5,
            font_size: 30,
            ..RenderOptions::default()
        };
        let generation = Generation::new(7);
        let tiny_grid = grid(5, 5, true, Sampling::Classic);
        let tiny = Renderer::new()
            .unwrap()
            .preview_task(
                Arc::clone(&tiny_grid),
                RenderOptions {
                    max_output_bytes: 100,
                    ..options
                },
                generation,
            )
            .unwrap();
        assert_eq!((tiny.raster.width, tiny.raster.height), (5, 5));
        assert!(Renderer::new()
            .unwrap()
            .preview_task(
                tiny_grid,
                RenderOptions {
                    max_output_bytes: 4,
                    ..options
                },
                generation,
            )
            .is_err());
        let mut task = Renderer::new()
            .unwrap()
            .preview_task(Arc::clone(&patterned), options, generation)
            .unwrap();
        assert_eq!(
            task.advance(0, generation),
            Err(ContractError::InvalidProgress)
        );
        assert_eq!(task.progress().completed(), 0);
        assert!(matches!(
            task.advance(1, generation),
            Ok(PreviewAdvance::InProgress(progress)) if progress.completed() == 1
        ));
        assert_eq!(task.finish(generation), Err(ContractError::InvalidProgress));

        let mut cancelled_before = Renderer::new()
            .unwrap()
            .preview_task(Arc::clone(&patterned), options, generation)
            .unwrap();
        cancelled_before.cancel();
        assert_eq!(
            cancelled_before.advance(1, generation),
            Err(ContractError::Cancelled)
        );
        assert_eq!(cancelled_before.raster.pixels.capacity(), 0);

        let mut cancelled_partial = Renderer::new()
            .unwrap()
            .preview_task(Arc::clone(&patterned), options, generation)
            .unwrap();
        cancelled_partial.advance(13, generation).unwrap();
        cancelled_partial.cancel();
        assert_eq!(
            cancelled_partial.advance(1, generation),
            Err(ContractError::Cancelled)
        );
        assert_eq!(cancelled_partial.raster.pixels.capacity(), 0);

        let mut stale = Renderer::new()
            .unwrap()
            .preview_task(Arc::clone(&patterned), options, generation)
            .unwrap();
        stale.advance(13, generation).unwrap();
        assert_eq!(
            stale.advance(1, Generation::new(8)),
            Err(ContractError::StaleGeneration)
        );
        assert_eq!(stale.raster.pixels.capacity(), 0);
        assert_eq!(
            stale.advance(1, generation),
            Err(ContractError::StaleGeneration)
        );

        let mut complete = Renderer::new()
            .unwrap()
            .preview_task(patterned, options, generation)
            .unwrap();
        complete.advance(u64::MAX, generation).unwrap();
        complete.cancel();
        assert_eq!(complete.finish(generation), Err(ContractError::Cancelled));
    }

    fn ramp_of(length: usize) -> Ramp {
        let text: String = (0..length)
            .map(|index| char::from(b'!' + u8::try_from(index).unwrap()))
            .collect();
        Ramp::new(&text).unwrap()
    }

    fn mode_options(mode: RenderMode, threshold: u8) -> RenderOptions {
        RenderOptions {
            sampling: Sampling::Average,
            mode,
            threshold,
            ..RenderOptions::default()
        }
    }

    fn glyph_at(luma: u8, options: &RenderOptions) -> char {
        cell_glyph(SampleCell::new(luma, [0; 4]), options)
    }

    #[test]
    fn ascii_endpoints_select_first_and_last_entries() {
        for length in [1, 2, 10, 64] {
            let ramp = ramp_of(length);
            let options = mode_options(RenderMode::Ascii(ramp), 128);
            let bytes = ramp.as_bytes();
            assert_eq!(
                glyph_at(0, &options),
                char::from(bytes[0]),
                "length {length}"
            );
            assert_eq!(
                glyph_at(255, &options),
                char::from(bytes[length - 1]),
                "length {length}"
            );
        }
    }

    #[test]
    fn ascii_index_is_monotone_over_every_luma_value() {
        for length in [1, 2, 3, 10, 64] {
            let ramp = ramp_of(length);
            let options = mode_options(RenderMode::Ascii(ramp), 128);
            let mut previous = 0;
            for luma in 0..=255 {
                let glyph = glyph_at(luma, &options);
                let index = ramp
                    .as_bytes()
                    .iter()
                    .position(|byte| char::from(*byte) == glyph)
                    .unwrap();
                assert!(index >= previous, "length {length} luma {luma}");
                previous = index;
            }
            assert_eq!(previous, length - 1);
        }
    }

    #[test]
    fn ascii_rounds_to_nearest_level_at_midpoints() {
        let ramp = Ramp::new("abc").unwrap();
        let options = mode_options(RenderMode::Ascii(ramp), 128);
        assert_eq!(glyph_at(63, &options), 'a');
        assert_eq!(glyph_at(64, &options), 'b');
        assert_eq!(glyph_at(191, &options), 'b');
        assert_eq!(glyph_at(192, &options), 'c');
        assert_eq!(glyph_at(254, &options), 'c');
    }

    #[test]
    fn binary_follows_threshold_boundary() {
        let options = mode_options(RenderMode::Binary, 128);
        assert_eq!(glyph_at(127, &options), '1');
        assert_eq!(glyph_at(128, &options), '0');
        assert_eq!(glyph_at(0, &options), '1');
        assert_eq!(glyph_at(255, &options), '0');
    }

    #[test]
    fn block_level_boundaries_are_nearest_rounded() {
        let options = mode_options(RenderMode::Block, 128);
        let expected = [
            (0, ' '),
            (31, ' '),
            (32, '\u{2591}'),
            (95, '\u{2591}'),
            (96, '\u{2592}'),
            (159, '\u{2592}'),
            (160, '\u{2593}'),
            (223, '\u{2593}'),
            (224, '\u{2588}'),
            (255, '\u{2588}'),
        ];
        for (luma, glyph) in expected {
            assert_eq!(glyph_at(luma, &options), glyph, "luma {luma}");
        }
    }

    #[test]
    fn block_coverage_matches_its_five_levels() {
        let coverage: Vec<u8> = BLOCK_LEVELS
            .iter()
            .map(|(glyph, _)| block_coverage(*glyph))
            .collect();
        assert_eq!(coverage, [0, 25, 50, 75, 100]);
        assert_eq!(block_coverage('x'), 0);
    }

    const GREEN: Rgb = Rgb {
        red: 0,
        green: 255,
        blue: 0,
    };
    const BLACK: Rgb = Rgb {
        red: 0,
        green: 0,
        blue: 0,
    };

    #[test]
    fn glyph_alpha_blend_is_exact_identity_on_green_over_black() {
        for alpha in 0..=255_u32 {
            let expected = u8::try_from(alpha).unwrap();
            assert_eq!(
                blend(GREEN, BLACK, alpha, 255).unwrap(),
                [0, expected, 0, 255]
            );
        }
    }

    #[test]
    fn block_blend_mixes_foreground_and_background_by_percent() {
        assert_eq!(blend(GREEN, BLACK, 0, 100).unwrap(), [0, 0, 0, 255]);
        assert_eq!(blend(GREEN, BLACK, 25, 100).unwrap(), [0, 64, 0, 255]);
        assert_eq!(blend(GREEN, BLACK, 50, 100).unwrap(), [0, 128, 0, 255]);
        assert_eq!(blend(GREEN, BLACK, 75, 100).unwrap(), [0, 191, 0, 255]);
        assert_eq!(blend(GREEN, BLACK, 100, 100).unwrap(), [0, 255, 0, 255]);
        assert_eq!(
            blend(GREEN, BLACK, 101, 100),
            Err(ContractError::ArithmeticOverflow)
        );
    }

    #[test]
    fn ascii_and_block_reject_non_average_sampling() {
        for mode in [RenderMode::Block, RenderMode::Ascii(Ramp::DEFAULT_ASCII)] {
            let options = RenderOptions {
                sampling: Sampling::Classic,
                mode,
                ..RenderOptions::default()
            };
            assert_eq!(
                options.validate().unwrap_err().to_string(),
                "invalid settings: render mode sampling"
            );
        }
        assert!(RenderOptions::default().validate().is_ok());
        assert!(RenderOptions {
            sampling: Sampling::Average,
            mode: RenderMode::Binary,
            ..RenderOptions::default()
        }
        .validate()
        .is_ok());
    }

    fn styled(style: RenderStyle, mode: RenderMode, threshold: u8) -> RenderOptions {
        RenderOptions {
            style,
            cell_size: 5,
            ..mode_options(mode, threshold)
        }
    }

    fn classic_cell_grid(color: [u8; 4]) -> Arc<crate::SampleGrid> {
        let source = source_pixels(5, 5, move |_, _| color);
        Sampler::new(source).sample(5, Sampling::Classic).unwrap()
    }

    fn source_pixels(
        width: u32,
        height: u32,
        pixel: impl Fn(u32, u32) -> [u8; 4],
    ) -> Arc<crate::DecodedSource> {
        let image = ImageBuffer::from_fn(width, height, |x, y| Rgba(pixel(x, y)));
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        Arc::new(decode(&bytes.into_inner(), DecodeLimits::default()).unwrap())
    }

    #[test]
    fn option_bounds_accept_boundaries_and_reject_just_outside() {
        let base = RenderOptions::default();
        for cell_size in [MIN_CELL_SIZE, MAX_CELL_SIZE] {
            assert!(RenderOptions { cell_size, ..base }.validate().is_ok());
        }
        for cell_size in [MIN_CELL_SIZE - 1, MAX_CELL_SIZE + 1] {
            assert_eq!(
                RenderOptions { cell_size, ..base }
                    .validate()
                    .unwrap_err()
                    .to_string(),
                "invalid settings: cell_size"
            );
        }
        for font_size in [MIN_FONT_SIZE, MAX_FONT_SIZE] {
            assert!(RenderOptions { font_size, ..base }.validate().is_ok());
        }
        for font_size in [MIN_FONT_SIZE - 1, MAX_FONT_SIZE + 1] {
            assert_eq!(
                RenderOptions { font_size, ..base }
                    .validate()
                    .unwrap_err()
                    .to_string(),
                "invalid settings: font_size"
            );
        }
        for contrast in [MIN_CONTRAST, MAX_CONTRAST] {
            let style = RenderStyle {
                contrast,
                ..RenderStyle::default()
            };
            assert!(RenderOptions { style, ..base }.validate().is_ok());
        }
        for contrast in [MIN_CONTRAST - 1, MAX_CONTRAST + 1] {
            let style = RenderStyle {
                contrast,
                ..RenderStyle::default()
            };
            assert_eq!(
                RenderOptions { style, ..base }
                    .validate()
                    .unwrap_err()
                    .to_string(),
                "invalid settings: contrast"
            );
        }
        for threshold in [MIN_THRESHOLD, MAX_THRESHOLD] {
            assert!(RenderOptions { threshold, ..base }.validate().is_ok());
        }
    }

    #[test]
    fn ramp_bound_accepts_sixty_four_entries_and_rejects_sixty_five() {
        let full = Ramp::new(&"a".repeat(MAX_RAMP_LENGTH)).unwrap();
        assert!(RenderOptions {
            sampling: Sampling::Average,
            mode: RenderMode::Ascii(full),
            ..RenderOptions::default()
        }
        .validate()
        .is_ok());
        assert_eq!(
            Ramp::new(&"a".repeat(MAX_RAMP_LENGTH + 1)),
            Err(ContractError::InvalidRamp)
        );
    }

    #[test]
    fn binary_threshold_boundary_flips_at_the_toned_value() {
        let options = mode_options(RenderMode::Binary, 128);
        assert_eq!(glyph_at(127, &options), '1');
        assert_eq!(glyph_at(128, &options), '0');
        let inverted = styled(
            RenderStyle {
                invert: true,
                ..RenderStyle::default()
            },
            RenderMode::Binary,
            128,
        );
        assert_eq!(glyph_at(128, &inverted), '1');
        assert_eq!(glyph_at(127, &inverted), '0');
        assert_eq!(glyph_at(255, &inverted), '1');
        assert_eq!(glyph_at(0, &inverted), '0');
    }

    #[test]
    fn contrast_is_applied_before_the_binary_threshold() {
        let flattened = styled(
            RenderStyle {
                contrast: -100,
                ..RenderStyle::default()
            },
            RenderMode::Binary,
            128,
        );
        for luma in [0, 127, 128, 255] {
            assert_eq!(glyph_at(luma, &flattened), '0', "luma {luma}");
        }
        let flattened_high = styled(
            RenderStyle {
                contrast: -100,
                ..RenderStyle::default()
            },
            RenderMode::Binary,
            129,
        );
        for luma in [0, 255] {
            assert_eq!(glyph_at(luma, &flattened_high), '1', "luma {luma}");
        }
    }

    #[test]
    fn invert_reverses_ramp_index_direction() {
        let ramp = Ramp::new("abc").unwrap();
        let normal = mode_options(RenderMode::Ascii(ramp), 128);
        let inverted = styled(
            RenderStyle {
                invert: true,
                ..RenderStyle::default()
            },
            RenderMode::Ascii(ramp),
            128,
        );
        assert_eq!(glyph_at(0, &normal), 'a');
        assert_eq!(glyph_at(255, &normal), 'c');
        assert_eq!(glyph_at(0, &inverted), 'c');
        assert_eq!(glyph_at(255, &inverted), 'a');
    }

    #[test]
    fn block_levels_follow_the_toned_value_under_invert() {
        let inverted = styled(
            RenderStyle {
                invert: true,
                ..RenderStyle::default()
            },
            RenderMode::Block,
            128,
        );
        assert_eq!(glyph_at(0, &inverted), '\u{2588}');
        assert_eq!(glyph_at(255, &inverted), ' ');
    }

    #[test]
    fn source_color_opaque_cell_renders_its_exact_rgb_at_full_block() {
        let grid = average_single_cell_grid([255, 230, 255, 255]);
        let options = styled(
            RenderStyle {
                color: ColorMode::Source,
                ..RenderStyle::default()
            },
            RenderMode::Block,
            128,
        );
        let frame = Renderer::new().unwrap().render(&grid, options).unwrap();
        assert!(frame
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel == [255, 230, 255, 255]));
    }

    #[test]
    fn source_color_transparent_cell_renders_exact_background() {
        let grid = average_single_cell_grid([255, 230, 255, 0]);
        let background = Rgb {
            red: 10,
            green: 20,
            blue: 30,
        };
        let options = styled(
            RenderStyle {
                color: ColorMode::Source,
                background,
                ..RenderStyle::default()
            },
            RenderMode::Block,
            128,
        );
        let frame = Renderer::new().unwrap().render(&grid, options).unwrap();
        assert!(frame
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel == [10, 20, 30, 255]));
    }

    #[test]
    fn source_color_half_alpha_matches_the_matte_formula_over_background() {
        let grid = average_single_cell_grid([255, 230, 255, 128]);
        let background = Rgb {
            red: 10,
            green: 20,
            blue: 30,
        };
        let options = styled(
            RenderStyle {
                color: ColorMode::Source,
                background,
                ..RenderStyle::default()
            },
            RenderMode::Block,
            128,
        );
        let frame = Renderer::new().unwrap().render(&grid, options).unwrap();
        let expected = [133, 125, 143, 255];
        assert!(frame
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel == expected));
    }

    fn average_single_cell_grid(color: [u8; 4]) -> Arc<crate::SampleGrid> {
        let cell_height = u32::from(
            Renderer::new()
                .unwrap()
                .aspect_geometry(5, 30)
                .unwrap()
                .source_cell_height,
        );
        let source = source_pixels(5, cell_height, move |_, _| color);
        Sampler::new(source)
            .sample_average_rect(5, cell_height)
            .unwrap()
    }

    #[test]
    fn source_color_is_opaque_and_background_bounded_for_every_mode() {
        let background = Rgb {
            red: 10,
            green: 20,
            blue: 30,
        };
        let style = RenderStyle {
            color: ColorMode::Source,
            background,
            ..RenderStyle::default()
        };
        let color = [255, 230, 255, 128];
        let average = average_single_cell_grid(color);
        let classic = classic_cell_grid(color);
        for (mode, grid, sampling) in [
            (RenderMode::Binary, classic, Sampling::Classic),
            (
                RenderMode::Ascii(Ramp::DEFAULT_ASCII),
                Arc::clone(&average),
                Sampling::Average,
            ),
            (RenderMode::Block, average, Sampling::Average),
        ] {
            let options = RenderOptions {
                sampling,
                style,
                cell_size: 5,
                ..mode_options(mode, 128)
            };
            let frame = Renderer::new().unwrap().render(&grid, options).unwrap();
            for pixel in frame.pixels().chunks_exact(4) {
                assert_eq!(pixel[3], 255, "{mode:?}");
                for (value, (low, high)) in pixel[..3].iter().zip([(10, 255), (20, 230), (30, 255)])
                {
                    assert!((low..=high).contains(value), "{mode:?} {pixel:?}");
                }
            }
        }
    }

    #[test]
    fn monochrome_block_extremes_are_exact_foreground_and_background() {
        let style = RenderStyle {
            foreground: Rgb {
                red: 255,
                green: 128,
                blue: 0,
            },
            background: Rgb {
                red: 0,
                green: 0,
                blue: 64,
            },
            ..RenderStyle::default()
        };
        let dark = average_single_cell_grid([0, 0, 0, 255]);
        let light = average_single_cell_grid([255, 255, 255, 255]);
        let options = styled(style, RenderMode::Block, 128);
        let mut renderer = Renderer::new().unwrap();
        let background = renderer.render(&dark, options).unwrap();
        assert!(background
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel == [0, 0, 64, 255]));
        let foreground = renderer.render(&light, options).unwrap();
        assert!(foreground
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel == [255, 128, 0, 255]));
    }

    #[test]
    fn monochrome_ignores_sampled_color_for_every_mode() {
        let grid = average_single_cell_grid([255, 230, 255, 255]);
        let style = RenderStyle {
            foreground: Rgb {
                red: 1,
                green: 2,
                blue: 3,
            },
            ..RenderStyle::default()
        };
        let mut renderer = Renderer::new().unwrap();
        let block = renderer
            .render(&grid, styled(style, RenderMode::Block, 128))
            .unwrap();
        assert!(block
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel == [1, 2, 3, 255]));
    }
}
